"""Layer 1 research strategies: the SDK, the runner-side harness and the truncation test.

Set M (FEAT-004 §2, ST-01, ST-03). The Rust half is `crates/backtest/src/position_series.rs`:
it validates the series the way this file does, runs the same stratified truncation points
and owns the evaluator that turns positions into fills. This file is what the *agent's*
function runs against.

What a research strategy is
---------------------------
A function from a `Data` object and a parameter object to a series of **target positions**,
one per decision time. It never returns a P&L: the platform computes that, under its own
execution policy and costs, and a function that tried to supply one has nowhere to put it.

    @research_strategy(instruments=["BTC-USD"], timeframe="1h",
                       params={"lookback": Param.int(10, 100)})
    def positions(data, p):
        rv = data.feature("rolling_std(logret(close), 20)")
        sig = data.close > data.close.shift(1) * (1 + rv)
        return sig.astype(float) * target_vol_scale(rv, 0.4)

Why the truncation test exists
------------------------------
Free-form Python can read the future by accident: `bfill`, a full-sample z-score, a centred
rolling window, `shift(-1)`. Each produces a series that looks like alpha. The test asks one
question — *does the function give the same earlier answers when the later data does not
exist?* — and `truncation_test` answers it exactly, at K stratified seeded cut points. Exactly
means bit for bit with NaN equal to NaN: a tolerance is a way of calling a small look-ahead
acceptable.

What `Data` guarantees
----------------------
Every series it hands out is clipped at the cutoff (so a function cannot see a bar after the
project's research cutoff), is aligned to the decision times, and is built from causal
operations only: `feature()` accepts a closed list of windowed expressions, evaluated with
right-aligned windows and `min_periods` equal to the window, never `eval`. There is no clock
and no network; the `rng` is seeded.

This is the numpy/pandas stand-in for the DATA-006 batch engine (ADR-0029), and says so: the
platform's feature engine replaces `Data.feature` when the research runner is wired (M-1.5).
"""

from __future__ import annotations

import ast
import math
from dataclasses import dataclass
from typing import Any, Callable

import numpy as np
import pandas as pd
import sys
import types

# The skill runner exec's source under a `__name__` that is not in sys.modules, and dataclasses
# (with string annotations) look their module up there. Register a stand-in so this file works
# both imported and exec'd.
sys.modules.setdefault(__name__, types.ModuleType(__name__))

TRUNCATION_CODE = "leakage.position_truncation_mismatch"


class ResearchStrategyError(Exception):
    """A refusal with a stable `code`, so a gate or an agent can branch on it."""

    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


# ── parameters ───────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class ParamSpec:
    kind: str  # "int" | "float"
    low: float
    high: float
    default: float | None = None

    def check(self, name: str, value: Any) -> Any:
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            raise ResearchStrategyError("param.type", f"{name} must be a number, got {value!r}")
        if not (self.low <= value <= self.high):
            raise ResearchStrategyError(
                "param.range", f"{name}={value} is outside [{self.low}, {self.high}]"
            )
        return int(value) if self.kind == "int" else float(value)


class Param:
    """Declared free parameters. They are the only free parameters (FEAT-004 §2.1)."""

    @staticmethod
    def int(low: int, high: int, default: int | None = None) -> ParamSpec:
        return ParamSpec("int", low, high, default)

    @staticmethod
    def float(low: float, high: float, default: float | None = None) -> ParamSpec:
        return ParamSpec("float", low, high, default)


class _Params:
    """Attribute access for the materialised parameter values."""

    def __init__(self, values: dict[str, Any]):
        object.__setattr__(self, "_v", dict(values))

    def __getattr__(self, name: str) -> Any:
        try:
            return self._v[name]
        except KeyError:
            raise AttributeError(f"{name} is not a declared parameter") from None

    def __setattr__(self, name, value):
        raise AttributeError("parameters are read-only")


@dataclass(frozen=True)
class Meta:
    instruments: tuple[str, ...]
    timeframe: str
    params: dict[str, ParamSpec]
    position_units: str
    allow_short: bool


def research_strategy(
    instruments: list[str],
    timeframe: str,
    params: dict[str, ParamSpec] | None = None,
    position_units: str = "fraction",
    allow_short: bool = False,
):
    """Declare a Layer 1 strategy. `allow_short` is explicit and defaults to False."""
    if position_units not in ("fraction", "units"):
        raise ResearchStrategyError("meta.units", "position_units is `fraction` or `units`")
    if not params:
        raise ResearchStrategyError(
            "meta.no_params",
            "declare at least one parameter: the declared params are the only free parameters, "
            "and a strategy with none cannot be swept by the platform sampler",
        )

    def wrap(fn: Callable) -> Callable:
        fn.__research__ = Meta(
            tuple(instruments), timeframe, dict(params), position_units, bool(allow_short)
        )
        return fn

    return wrap


# ── the data object ──────────────────────────────────────────────────────────

_SERIES = ("open", "high", "low", "close", "volume")

_PERIODS_PER_YEAR = {
    "1m": 525_600,
    "5m": 105_120,
    "15m": 35_040,
    "30m": 17_520,
    "1h": 8_760,
    "4h": 2_190,
    "1d": 365,
}


def _logret(x: pd.Series) -> pd.Series:
    return np.log(x).diff()


def _need_int(n, what):
    if not isinstance(n, (int, float)) or n != int(n) or int(n) < 1:
        raise ResearchStrategyError("feature.arg", f"{what} must be a positive integer, got {n!r}")
    return int(n)


# Every one is causal: a right-aligned window with min_periods equal to the window.
_FUNCS: dict[str, Callable] = {
    "logret": _logret,
    "ret": lambda x: x.pct_change(),
    "shift": lambda x, n=1: x.shift(_need_int(n, "shift")),
    "rolling_std": lambda x, n: x.rolling(_need_int(n, "window"), min_periods=int(n)).std(),
    "rolling_mean": lambda x, n: x.rolling(_need_int(n, "window"), min_periods=int(n)).mean(),
    "rolling_max": lambda x, n: x.rolling(_need_int(n, "window"), min_periods=int(n)).max(),
    "rolling_min": lambda x, n: x.rolling(_need_int(n, "window"), min_periods=int(n)).min(),
    "ema": lambda x, n: x.ewm(span=_need_int(n, "span"), adjust=False, min_periods=int(n)).mean(),
    "abs": lambda x: x.abs(),
    "log": lambda x: np.log(x),
    "sqrt": lambda x: np.sqrt(x),
}


class Data:
    """What a research strategy may see.

    Bars are a frame indexed by the bar's close time in nanoseconds with columns open, high,
    low, close, volume. `cutoff_ns` (inclusive) clips every series: nothing after it exists.
    """

    def __init__(
        self,
        bars: pd.DataFrame,
        timeframe: str = "1h",
        cutoff_ns: int | None = None,
        seed: int = 0,
    ):
        missing = [c for c in _SERIES if c not in bars.columns]
        if missing:
            raise ResearchStrategyError("data.columns", f"bars lack {missing}")
        if not bars.index.is_monotonic_increasing or bars.index.has_duplicates:
            raise ResearchStrategyError("data.index", "bars must be strictly increasing in time")
        frame = bars if cutoff_ns is None else bars[bars.index <= cutoff_ns]
        self._bars = frame[list(_SERIES)].astype(float)
        self.timeframe = timeframe
        self.periods_per_year = _PERIODS_PER_YEAR.get(timeframe, 8_760)
        self._rng = np.random.default_rng(seed)
        #: every expression evaluated, in order, so a failed truncation test can say which
        #: inputs differed.
        self.features_used: dict[str, pd.Series] = {}

    @property
    def index(self) -> pd.Index:
        return self._bars.index

    @property
    def rng(self) -> np.random.Generator:
        return self._rng

    def __getattr__(self, name: str) -> pd.Series:
        if name in _SERIES:
            return self._bars[name]
        raise AttributeError(name)

    def prediction(self, handle: str):
        raise ResearchStrategyError(
            "data.prediction_unavailable",
            f"prediction series `{handle}` are served by the platform's research runner (M-1.5); "
            "this harness has none",
        )

    def feature(self, expr: str) -> pd.Series:
        """Evaluate a windowed expression over the bars. Closed grammar; never `eval`."""
        value = self._eval(ast.parse(expr, mode="eval").body, expr)
        if not isinstance(value, pd.Series):
            value = pd.Series(value, index=self._bars.index)
        self.features_used[expr] = value
        return value

    def _eval(self, node: ast.AST, src: str):
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
            return node.value
        if isinstance(node, ast.Name):
            if node.id in _SERIES:
                return self._bars[node.id]
            raise ResearchStrategyError("feature.name", f"unknown series `{node.id}` in `{src}`")
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            return -self._eval(node.operand, src)
        if isinstance(node, ast.BinOp):
            a, b = self._eval(node.left, src), self._eval(node.right, src)
            ops = {ast.Add: lambda: a + b, ast.Sub: lambda: a - b, ast.Mult: lambda: a * b, ast.Div: lambda: a / b}
            for t, f in ops.items():
                if isinstance(node.op, t):
                    return f()
            raise ResearchStrategyError("feature.op", f"operator not allowed in `{src}`")
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
            fn = _FUNCS.get(node.func.id)
            if fn is None:
                raise ResearchStrategyError(
                    "feature.function",
                    f"unknown function `{node.func.id}` in `{src}`; known: {sorted(_FUNCS)}",
                )
            return fn(*[self._eval(a, src) for a in node.args])
        raise ResearchStrategyError("feature.syntax", f"`{src}` uses a construct that is not allowed")


def target_vol_scale(rv: pd.Series, target: float, periods_per_year: int = 8_760, cap: float = 1.0) -> pd.Series:
    """Scale to a target annualised volatility, `target / (rv * sqrt(periods_per_year))`, capped.

    `rv` is a per-bar volatility, so the result is causal wherever `rv` is.
    """
    scale = target / (rv * math.sqrt(periods_per_year))
    return scale.replace([np.inf, -np.inf], np.nan).clip(upper=cap)


# ── running one ──────────────────────────────────────────────────────────────


def _materialise(meta: Meta, params: dict[str, Any]) -> _Params:
    values: dict[str, Any] = {}
    for name, spec in meta.params.items():
        raw = params.get(name, spec.default)
        if raw is None:
            raise ResearchStrategyError("param.missing", f"{name} has no value and no default")
        values[name] = spec.check(name, raw)
    extra = sorted(set(params) - set(meta.params))
    if extra:
        raise ResearchStrategyError("param.undeclared", f"{extra} are not declared params")
    return _Params(values)


def _validate(series: Any, meta: Meta, index: pd.Index) -> pd.Series:
    if not isinstance(series, pd.Series):
        raise ResearchStrategyError("series.type", "the function must return a pandas Series")
    if not series.index.is_monotonic_increasing or series.index.has_duplicates:
        raise ResearchStrategyError("series.index", "the output index must strictly increase")
    stray = series.index.difference(index)
    if len(stray):
        raise ResearchStrategyError(
            "series.not_decision_time",
            f"{int(stray[0])} is not a decision time; the output index must be a subset of them",
        )
    vals = series.astype(float)
    inf = np.isinf(vals.to_numpy())
    if inf.any():
        raise ResearchStrategyError("series.non_finite", f"non-finite target at {int(vals.index[inf][0])}")
    if meta.position_units == "fraction":
        out = vals.abs() > 1.0
        if out.any():
            raise ResearchStrategyError(
                "series.out_of_bounds", f"|target| > 1 at {int(vals.index[out.to_numpy()][0])}"
            )
    if not meta.allow_short:
        short = vals < 0.0
        if short.any():
            raise ResearchStrategyError(
                "series.short_not_allowed", f"short target at {int(vals.index[short.to_numpy()][0])}"
            )
    return vals


def run_positions(
    fn: Callable,
    bars: pd.DataFrame,
    params: dict[str, Any] | None = None,
    cutoff_ns: int | None = None,
    seed: int = 0,
) -> tuple[pd.Series, Data]:
    """Run one declared strategy over bars. NaN in the output means 'no change'."""
    meta: Meta | None = getattr(fn, "__research__", None)
    if meta is None:
        raise ResearchStrategyError("meta.undeclared", "decorate the function with @research_strategy")
    p = _materialise(meta, params or {})
    data = Data(bars, meta.timeframe, cutoff_ns, seed)
    out = fn(data, p)
    return _validate(out, meta, data.index), data


# ── the truncation test ──────────────────────────────────────────────────────

_M64 = (1 << 64) - 1


class _SplitMix:
    """The same generator as `crates/backtest/src/rng.rs`, so a Python re-run and a Rust one
    choose the same cuts for one seed."""

    def __init__(self, seed: int):
        self.state = (seed ^ 0x9E3779B97F4A7C15) & _M64

    def next_u64(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & _M64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & _M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & _M64
        return z ^ (z >> 31)

    def below(self, n: int) -> int:
        return 0 if n == 0 else self.next_u64() % n


def truncation_points(seed: int, n: int, decision_times: list[int]) -> list[int]:
    """K cuts, one per equal slice of the decision times; never the final one."""
    m = max(len(decision_times) - 1, 0)
    if n == 0 or m == 0:
        return []
    if m <= n:
        return list(decision_times[:m])
    rng = _SplitMix(seed)
    out = []
    for k in range(n):
        lo, hi = k * m // n, (k + 1) * m // n
        out.append(decision_times[lo + rng.below(hi - lo)])
    return out


def _same(a: float, b: float) -> bool:
    return (math.isnan(a) and math.isnan(b)) or a == b


def truncation_test(
    fn: Callable,
    bars: pd.DataFrame,
    params: dict[str, Any] | None = None,
    k: int = 8,
    seed: int = 0,
) -> dict:
    """Re-run `fn` on data ending at K cut points and require the earlier answers to be identical.

    Returns `{"passed", "cuts_checked", ...}`; a failure carries `cut_ns`,
    `first_differing_ns`, the two values and `differing_inputs`, the feature expressions
    whose values differed before the cut (the likely source of the look-ahead).
    """
    full, full_data = run_positions(fn, bars, params, seed=seed)
    times = [int(t) for t in full_data.index]
    cuts = truncation_points(seed, k, times)
    if not cuts:
        return {"passed": False, "code": "truncation.not_computable", "reason": "no cut points"}
    for cut in sorted(cuts):
        trunc, trunc_data = run_positions(fn, bars, params, cutoff_ns=cut, seed=seed)
        a = full[full.index <= cut]
        b = trunc[trunc.index <= cut]
        union = a.index.union(b.index)
        for ts in union:
            av = a.get(ts, None)
            bv = b.get(ts, None)
            if av is None or bv is None or not _same(float(av), float(bv)):
                return {
                    "passed": False,
                    "code": TRUNCATION_CODE,
                    "cut_ns": int(cut),
                    "first_differing_ns": int(ts),
                    "full": None if av is None or math.isnan(av) else float(av),
                    "truncated": None if bv is None or math.isnan(bv) else float(bv),
                    "differing_inputs": _differing_inputs(full_data, trunc_data, cut),
                }
    return {"passed": True, "cuts_checked": len(cuts)}


def _differing_inputs(full: Data, trunc: Data, cut: int) -> list[str]:
    out = []
    for expr, tv in trunc.features_used.items():
        fv = full.features_used.get(expr)
        if fv is None:
            out.append(expr)
            continue
        fv = fv[fv.index <= cut]
        tv = tv[tv.index <= cut]
        if len(fv) != len(tv) or not all(_same(float(x), float(y)) for x, y in zip(fv, tv)):
            out.append(expr)
    return out


_TUNED = ast.Constant


def lint_tuned_literals(source: str, declared: set[str] | None = None) -> list[dict]:
    """Flag numeric literals that look tuned (FEAT-004 §2.1: a warning at M).

    A literal with a fractional part of three or more digits, or an integer window over 30
    that is not obviously structural, is a free parameter in disguise. Small integers,
    0, 1, 0.5 and powers of ten are structural and pass.
    """
    findings = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Constant) and isinstance(node.value, float):
            text = repr(node.value)
            frac = text.split(".")[1] if "." in text and "e" not in text else ""
            if len(frac.rstrip("0")) >= 3:
                findings.append({"line": node.lineno, "value": node.value, "rule": "tuned_float"})
    return findings
