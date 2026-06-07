"""Universal rule-based strategy schema (FB-AP-XXX strategy builder).

A small, **import-free** schema that the strategy-builder UI (sentence/block builder, node
graph, and code-mode editors alike — see ``control_plane/strategy_builder_page.py``) compiles
down to, and that :class:`strategies.rule_based_strategy.RuleBasedStrategy` interprets at
runtime. Keeping this module free of ``nautilus_trader`` imports means it stays usable for
listing/validating/explaining strategies even when the optional ``backtest_nautilus`` extra
isn't installed (mirrors :mod:`strategies.registry`).

Shape (JSON-serialisable, see :meth:`RuleStrategySpec.to_dict` / :meth:`from_dict`)::

    {
      "indicators": [{"id": "ema_fast", "kind": "ema", "period": 7}, ...],
      "entry": {
        "side": "buy",
        "all": [{"type": "cross_above", "left": "ema_fast", "right": "ema_slow"}],
        "any": []
      },
      "size": {"type": "percent_of_equity", "value": 0.02},
      "exits": [{"type": "stop_loss", "value": 0.015}, {"type": "take_profit", "value": 0.04}]
    }

Every spec can render a plain-English :meth:`RuleStrategySpec.explain` so a user can sanity
check what they built before risking money on it — "every strategy should generate a
human-readable explanation" was an explicit product requirement.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from typing import Any

# ---------------------------------------------------------------------------
# Indicators
# ---------------------------------------------------------------------------

#: Indicator kinds supported by :class:`strategies.rule_based_strategy.RuleBasedStrategy`.
#: Each maps to a single-period Nautilus indicator (``Indicator(period)``) — kept to this
#: subset for v1 so the builder UI's "pick an indicator" step stays simple; multi-parameter
#: indicators (MACD, Bollinger Bands, VWAP, ...) are natural follow-ups once the simple
#: single-period set has proven the round trip end to end.
INDICATOR_KINDS: dict[str, str] = {
    "ema": "EMA (exponential moving average)",
    "sma": "SMA (simple moving average)",
    "rsi": "RSI (relative strength index)",
    "atr": "ATR (average true range)",
}

#: Condition types the engine knows how to evaluate, with a human-readable verb phrase.
CONDITION_TYPES: dict[str, str] = {
    "cross_above": "crosses above",
    "cross_below": "crosses below",
    "greater_than": "is greater than",
    "less_than": "is less than",
    "rising": "is rising",
    "falling": "is falling",
}

#: Position-sizing modes.
SIZE_TYPES: dict[str, str] = {
    "percent_of_equity": "percent of account equity",
    "fixed_quantity": "fixed quantity",
}

#: Exit-rule types.
EXIT_TYPES: dict[str, str] = {
    "stop_loss": "stop loss",
    "take_profit": "take profit",
    "trailing_stop": "trailing stop",
}

#: Trade direction for the entry rule.
SIDES: dict[str, str] = {"buy": "buy", "sell": "sell"}


class RuleSpecError(ValueError):
    """A rule_spec failed validation; ``str(exc)`` is a user-facing explanation."""


@dataclass(frozen=True)
class IndicatorSpec:
    """One named indicator the strategy computes from bars (referenced by ``id``)."""

    id: str
    kind: str
    period: int = 14

    def validate(self) -> None:
        if not self.id or not self.id.strip():
            raise RuleSpecError("every indicator needs a name")
        if self.kind not in INDICATOR_KINDS:
            raise RuleSpecError(f"unknown indicator kind {self.kind!r}")
        if self.period < 1 or self.period > 1000:
            raise RuleSpecError(f"indicator {self.id!r}: period must be between 1 and 1000")

    def label(self) -> str:
        return f"{self.period}-period {INDICATOR_KINDS[self.kind]}"

    def to_dict(self) -> dict[str, Any]:
        return {"id": self.id, "kind": self.kind, "period": self.period}

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "IndicatorSpec":
        return IndicatorSpec(
            id=str(raw.get("id", "")),
            kind=str(raw.get("kind", "")),
            period=int(raw.get("period", 14)),
        )


@dataclass(frozen=True)
class Condition:
    """One trigger condition: ``<left> <type> <right>``.

    ``left`` is always an indicator id (or ``"price"``); ``right`` is either another
    indicator id or a literal numeric threshold (mutually exclusive with ``right_id``).
    """

    type: str
    left: str
    right_id: str | None = None
    right_value: float | None = None

    def validate(self, indicator_ids: set[str]) -> None:
        if self.type not in CONDITION_TYPES:
            raise RuleSpecError(f"unknown condition type {self.type!r}")
        if self.left != "price" and self.left not in indicator_ids:
            raise RuleSpecError(f"condition references unknown indicator {self.left!r}")
        if self.type in ("rising", "falling"):
            return  # unary — no right-hand side
        if (self.right_id is None) == (self.right_value is None):
            raise RuleSpecError(
                f"condition on {self.left!r} needs exactly one of right_id / right_value"
            )
        if self.right_id is not None and self.right_id not in indicator_ids and self.right_id != "price":
            raise RuleSpecError(f"condition references unknown indicator {self.right_id!r}")

    def _right_label(self, indicators_by_id: dict[str, "IndicatorSpec"]) -> str:
        if self.right_id is not None:
            spec = indicators_by_id.get(self.right_id)
            return spec.label() if spec else self.right_id
        return f"{self.right_value:g}"

    def _left_label(self, indicators_by_id: dict[str, "IndicatorSpec"]) -> str:
        if self.left == "price":
            return "the price"
        spec = indicators_by_id.get(self.left)
        return spec.label() if spec else self.left

    def explain(self, indicators_by_id: dict[str, "IndicatorSpec"]) -> str:
        verb = CONDITION_TYPES[self.type]
        left = self._left_label(indicators_by_id)
        if self.type in ("rising", "falling"):
            return f"{left} {verb}"
        return f"{left} {verb} {self._right_label(indicators_by_id)}"

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {"type": self.type, "left": self.left}
        if self.right_id is not None:
            out["right_id"] = self.right_id
        if self.right_value is not None:
            out["right_value"] = self.right_value
        return out

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "Condition":
        return Condition(
            type=str(raw.get("type", "")),
            left=str(raw.get("left", "")),
            right_id=raw.get("right_id"),
            right_value=(float(raw["right_value"]) if raw.get("right_value") is not None else None),
        )


@dataclass(frozen=True)
class EntryRule:
    """Trade direction plus the conditions that must hold for entry.

    ``all_of`` conditions must ALL be true (AND); ``any_of`` conditions need only ONE true
    (OR). Both groups are combined with AND when both are non-empty — i.e. "all of these,
    and at least one of those".
    """

    side: str
    all_of: tuple[Condition, ...] = field(default_factory=tuple)
    any_of: tuple[Condition, ...] = field(default_factory=tuple)

    def validate(self, indicator_ids: set[str]) -> None:
        if self.side not in SIDES:
            raise RuleSpecError(f"unknown entry side {self.side!r}")
        if not self.all_of and not self.any_of:
            raise RuleSpecError("entry rule needs at least one condition")
        for cond in (*self.all_of, *self.any_of):
            cond.validate(indicator_ids)

    def explain(self, indicators_by_id: dict[str, "IndicatorSpec"]) -> str:
        parts: list[str] = []
        if self.all_of:
            parts.append(" and ".join(c.explain(indicators_by_id) for c in self.all_of))
        if self.any_of:
            parts.append("(" + " or ".join(c.explain(indicators_by_id) for c in self.any_of) + ")")
        condition_text = " and ".join(parts)
        return f"{SIDES[self.side].capitalize()} when {condition_text}"

    def to_dict(self) -> dict[str, Any]:
        return {
            "side": self.side,
            "all": [c.to_dict() for c in self.all_of],
            "any": [c.to_dict() for c in self.any_of],
        }

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "EntryRule":
        return EntryRule(
            side=str(raw.get("side", "")),
            all_of=tuple(Condition.from_dict(c) for c in raw.get("all", [])),
            any_of=tuple(Condition.from_dict(c) for c in raw.get("any", [])),
        )


@dataclass(frozen=True)
class SizeRule:
    """How large each entry should be."""

    type: str = "percent_of_equity"
    value: float = 0.02

    def validate(self) -> None:
        if self.type not in SIZE_TYPES:
            raise RuleSpecError(f"unknown size type {self.type!r}")
        if self.value <= 0:
            raise RuleSpecError("size value must be positive")
        if self.type == "percent_of_equity" and self.value > 1:
            raise RuleSpecError("percent_of_equity must be a fraction (e.g. 0.02 for 2%), not > 1")

    def explain(self) -> str:
        if self.type == "percent_of_equity":
            return f"sized at {self.value * 100:g}% of account equity"
        return f"sized at a fixed {self.value:g} units"

    def to_dict(self) -> dict[str, Any]:
        return {"type": self.type, "value": self.value}

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "SizeRule":
        return SizeRule(type=str(raw.get("type", "percent_of_equity")), value=float(raw.get("value", 0.02)))


@dataclass(frozen=True)
class ExitRule:
    """One exit condition (stop loss / take profit / trailing stop), as a fraction move."""

    type: str
    value: float

    def validate(self) -> None:
        if self.type not in EXIT_TYPES:
            raise RuleSpecError(f"unknown exit type {self.type!r}")
        if self.value <= 0 or self.value > 1:
            raise RuleSpecError(f"{self.type} value must be a fraction between 0 and 1 (e.g. 0.02 for 2%)")

    def explain(self) -> str:
        return f"{EXIT_TYPES[self.type]} at {self.value * 100:g}%"

    def to_dict(self) -> dict[str, Any]:
        return {"type": self.type, "value": self.value}

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "ExitRule":
        return ExitRule(type=str(raw.get("type", "")), value=float(raw.get("value", 0)))


@dataclass(frozen=True)
class RuleStrategySpec:
    """A complete user-built strategy: indicators + entry rule + sizing + exits."""

    name: str
    indicators: tuple[IndicatorSpec, ...] = field(default_factory=tuple)
    entry: EntryRule | None = None
    size: SizeRule = field(default_factory=SizeRule)
    exits: tuple[ExitRule, ...] = field(default_factory=tuple)

    def indicators_by_id(self) -> dict[str, IndicatorSpec]:
        return {ind.id: ind for ind in self.indicators}

    def validate(self) -> None:
        if not self.name or not self.name.strip():
            raise RuleSpecError("strategy needs a name")
        if not self.indicators:
            raise RuleSpecError("strategy needs at least one indicator")
        ids = [ind.id for ind in self.indicators]
        if len(ids) != len(set(ids)):
            raise RuleSpecError("indicator names must be unique")
        for ind in self.indicators:
            ind.validate()
        if self.entry is None:
            raise RuleSpecError("strategy needs an entry rule")
        self.entry.validate(set(ids))
        self.size.validate()
        if not self.exits:
            raise RuleSpecError("strategy needs at least one exit rule (stop loss and/or take profit)")
        for exit_rule in self.exits:
            exit_rule.validate()

    def explain(self) -> str:
        """Plain-English description of the whole strategy, for the builder UI's preview pane."""
        if self.entry is None:
            return f"“{self.name}” has no entry rule yet."
        by_id = self.indicators_by_id()
        sentences = [
            f"{self.entry.explain(by_id)}, {self.size.explain()}.",
        ]
        if self.exits:
            sentences.append("Exit on " + ", or ".join(e.explain() for e in self.exits) + ".")
        return " ".join(sentences)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "indicators": [ind.to_dict() for ind in self.indicators],
            "entry": self.entry.to_dict() if self.entry else None,
            "size": self.size.to_dict(),
            "exits": [e.to_dict() for e in self.exits],
        }

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), separators=(",", ":"), sort_keys=True)

    @staticmethod
    def from_dict(raw: dict[str, Any]) -> "RuleStrategySpec":
        entry_raw = raw.get("entry")
        return RuleStrategySpec(
            name=str(raw.get("name", "")),
            indicators=tuple(IndicatorSpec.from_dict(i) for i in raw.get("indicators", [])),
            entry=(EntryRule.from_dict(entry_raw) if isinstance(entry_raw, dict) else None),
            size=SizeRule.from_dict(raw.get("size", {})),
            exits=tuple(ExitRule.from_dict(e) for e in raw.get("exits", [])),
        )

    @staticmethod
    def from_json(raw: str) -> "RuleStrategySpec":
        return RuleStrategySpec.from_dict(json.loads(raw))

    def renamed(self, name: str) -> "RuleStrategySpec":
        return replace(self, name=name)
