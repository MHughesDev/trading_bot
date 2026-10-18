"""The Layer 1 SDK and truncation test (Set M, M-1.4). Run: python -m pytest apps/skill-runner/tests"""

import math
import os
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import research_strategy as rs  # noqa: E402
from research_strategy import Param, research_strategy, target_vol_scale  # noqa: E402

HOUR = 3_600_000_000_000

# Pinned from `crates/backtest/src/position_series.rs::cuts_match_the_python_harness`: one
# generator (SplitMix64) and one slicing rule in both languages, so a Rust re-run and a Python
# one cut at the same bars for one seed. Change one side and the other test fails.
RUST_CUTS = [5, 133, 368, 478, 515, 630, 802, 966]


def make_bars(n=400, seed=5):
    r = np.random.default_rng(seed)
    close = 100 * np.exp(np.cumsum(r.normal(0, 0.01, n)))
    open_ = np.concatenate([[100.0], close[:-1]])
    high = np.maximum(open_, close) * (1 + np.abs(r.normal(0, 0.002, n)))
    low = np.minimum(open_, close) * (1 - np.abs(r.normal(0, 0.002, n)))
    vol = r.uniform(10, 20, n)
    idx = (np.arange(n) + 1) * HOUR
    return pd.DataFrame({"open": open_, "high": high, "low": low, "close": close, "volume": vol}, index=idx)


PARAMS = {"lookback": Param.int(10, 100, 40), "k": Param.float(0.5, 3.0, 1.0)}


@research_strategy(instruments=["BTC-USD"], timeframe="1h", params=PARAMS)
def example(data, p):
    rv = data.feature("rolling_std(logret(close), 20)")
    data.feature(f"rolling_max(high, {p.lookback}) - rolling_min(low, {p.lookback})")
    sig = data.close > data.close.shift(1) * (1 + p.k * rv)
    return sig.astype(float) * target_vol_scale(rv, 0.4)


@research_strategy(instruments=["BTC-USD"], timeframe="1h", params=PARAMS)
def bfilled(data, p):
    # A feature with gaps, back-filled: each hole takes the NEXT bar's value.
    v = data.volume.where(np.arange(len(data.volume)) % 2 == 0)
    data.feature("rolling_mean(close, 5)")
    return (v.bfill() > 15).astype(float)


@research_strategy(instruments=["BTC-USD"], timeframe="1h", params=PARAMS)
def full_sample_mean(data, p):
    rv = data.feature("rolling_std(logret(close), 20)")
    return (rv.fillna(rv.mean()) > rv.mean()).astype(float)


@research_strategy(instruments=["BTC-USD"], timeframe="1h", params=PARAMS)
def shifted_forward(data, p):
    return (data.close.shift(-1) > data.close).astype(float)


def test_the_feature_4_example_runs_and_stays_in_bounds():
    out, _ = rs.run_positions(example, make_bars())
    assert out.abs().max() <= 1.0
    assert out.notna().sum() > 300
    assert out.index.is_monotonic_increasing


def test_a_causal_strategy_passes_the_truncation_test():
    v = rs.truncation_test(example, make_bars(), k=8, seed=3)
    assert v == {"passed": True, "cuts_checked": 8}


def test_a_bfill_variant_fails_naming_the_first_differing_timestamp():
    v = rs.truncation_test(bfilled, make_bars(), k=8, seed=3)
    assert v["passed"] is False
    assert v["code"] == rs.TRUNCATION_CODE
    assert v["first_differing_ns"] <= v["cut_ns"]
    assert v["first_differing_ns"] % HOUR == 0


def test_a_full_sample_statistic_and_a_forward_shift_both_fail():
    for fn in (full_sample_mean, shifted_forward):
        v = rs.truncation_test(fn, make_bars(), k=8, seed=3)
        assert v["passed"] is False, fn.__name__
    # The feature grammar is causal by construction, so a look-ahead in the function's own code
    # is not blamed on a feature: `differing_inputs` lists only inputs that really differed.
    v = rs.truncation_test(full_sample_mean, make_bars(), k=8, seed=3)
    assert v["differing_inputs"] == []


def test_a_difference_by_one_ulp_is_a_failure():
    @research_strategy(instruments=["X"], timeframe="1h", params=PARAMS)
    def one_ulp(data, p):
        s = (data.close > data.close.shift(1)).astype(float) * 0.5
        # When the data is truncated the 3rd value is one ulp off.
        if len(s) < 399:
            s.iloc[2] = np.nextafter(s.iloc[2], 1.0)
        return s

    v = rs.truncation_test(one_ulp, make_bars(), k=8, seed=3)
    assert v["passed"] is False


def test_nan_is_no_change_and_nan_equals_nan():
    assert rs._same(float("nan"), float("nan"))
    assert not rs._same(float("nan"), 0.0)
    out, _ = rs.run_positions(example, make_bars())
    assert out.isna().any()  # warm-up NaN is allowed and means "no change"


def test_data_never_exposes_a_bar_after_the_cutoff():
    bars = make_bars()
    cut = int(bars.index[99])
    d = rs.Data(bars, cutoff_ns=cut)
    assert d.close.index.max() == cut
    assert len(d.close) == 100
    with pytest.raises(rs.ResearchStrategyError) as e:
        d.feature("close.shift(-1)")
    assert e.value.code in {"feature.syntax", "feature.function"}


def test_feature_expressions_are_a_closed_grammar_and_never_eval():
    d = rs.Data(make_bars())
    for bad in ("__import__('os').system('x')", "close ** 2", "open_file(close)", "foo"):
        with pytest.raises(rs.ResearchStrategyError):
            d.feature(bad)
    assert d.feature("shift(close, 1)").iloc[1] == d.close.iloc[0]


def test_two_runs_with_one_seed_are_identical():
    @research_strategy(instruments=["X"], timeframe="1h", params=PARAMS)
    def noisy(data, p):
        return pd.Series(data.rng.uniform(0, 1, len(data.index)), index=data.index)

    a, _ = rs.run_positions(noisy, make_bars(), seed=11)
    b, _ = rs.run_positions(noisy, make_bars(), seed=11)
    c, _ = rs.run_positions(noisy, make_bars(), seed=12)
    assert a.equals(b)
    assert not a.equals(c)


def test_declared_params_are_the_only_free_parameters():
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(example, make_bars(), {"lookback": 500})
    assert e.value.code == "param.range"
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(example, make_bars(), {"lookback": 20, "mystery": 1})
    assert e.value.code == "param.undeclared"
    with pytest.raises(rs.ResearchStrategyError) as e:
        research_strategy(instruments=["X"], timeframe="1h", params={})
    assert e.value.code == "meta.no_params"


def test_the_output_is_validated():
    def declared(body, **kw):
        return research_strategy(instruments=["X"], timeframe="1h", params=PARAMS, **kw)(body)

    bars = make_bars(60)
    stray = declared(lambda d, p: pd.Series([0.1], index=[7]))
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(stray, bars)
    assert e.value.code == "series.not_decision_time"
    loud = declared(lambda d, p: d.close * 0 + 2.0)
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(loud, bars)
    assert e.value.code == "series.out_of_bounds"
    short = declared(lambda d, p: d.close * 0 - 0.5)
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(short, bars)
    assert e.value.code == "series.short_not_allowed"
    ok = declared(lambda d, p: d.close * 0 - 0.5, allow_short=True)
    assert rs.run_positions(ok, bars)[0].iloc[0] == -0.5
    inf = declared(lambda d, p: d.close * np.inf)
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(inf, bars)
    assert e.value.code == "series.non_finite"


def test_an_undecorated_function_is_refused():
    with pytest.raises(rs.ResearchStrategyError) as e:
        rs.run_positions(lambda d, p: None, make_bars())
    assert e.value.code == "meta.undeclared"


def test_target_vol_scale_is_capped_and_causal():
    rv = pd.Series([0.0, 0.001, 0.01, np.nan])
    s = target_vol_scale(rv, 0.4)
    assert s.iloc[1] == 1.0  # capped
    assert s.iloc[2] == pytest.approx(0.4 / (0.01 * math.sqrt(8760)))
    assert math.isnan(s.iloc[0]) and math.isnan(s.iloc[3])  # 0 vol is undefined, not infinite


def test_cut_points_are_stratified_seeded_and_never_the_last():
    times = list(range(1000))
    a = rs.truncation_points(7, 8, times)
    assert a == rs.truncation_points(7, 8, times)
    assert len(a) == 8 and 999 not in a
    for k, c in enumerate(a):
        assert k * 999 // 8 <= c < (k + 1) * 999 // 8
    assert rs.truncation_points(1, 8, [10, 20, 30]) == [10, 20]
    assert rs.truncation_points(1, 8, [10]) == []


def test_cut_points_match_the_rust_implementation():
    assert RUST_CUTS is not None, "pin the Rust cuts here"
    assert rs.truncation_points(7, 8, list(range(1000))) == RUST_CUTS


def test_tuned_literals_are_linted():
    src = "x = a * 0.0137\ny = b * 0.5\nz = c + 2\n"
    found = rs.lint_tuned_literals(src)
    assert [f["line"] for f in found] == [1]
