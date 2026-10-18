"""The `layer1_run` harness (Set M M-1.5): research_strategy.py + layer1_driver.py concatenated and
exec'd under the skill runner's `__skill__` name, the way the platform runs it.

What it proves that the Rust tests cannot: the composed source runs under the runner's namespace,
returns the full series and one re-run per cut, refuses a function that declares no params or the
wrong instrument/timeframe with a stable code, and hands the declared params to the sweep.
"""

import os

import numpy as np
import pandas as pd

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
SRC = (
    open(os.path.join(ROOT, "apps", "skill-runner", "research_strategy.py"), encoding="utf-8").read()
    + "\n"
    + open(os.path.join(ROOT, "apps", "skill-runner", "layer1_driver.py"), encoding="utf-8").read()
)

CAUSAL = '''
@research_strategy(instruments=["BTC-USD"], timeframe="1h", params={"lb": Param.int(2, 50, 5), "k": Param.float(0.5, 3.0, 1.0)})
def positions(data, p):
    return (data.close > data.close.shift(p.lb)).astype(float)
'''


def driver():
    ns = {"__name__": "__skill__"}
    exec(compile(SRC, "<layer1>", "exec"), ns)
    return ns["run"]


def bars_file(tmp_path, n=300):
    close = 100 + np.cumsum(np.random.default_rng(1).normal(0, 1, n))
    df = pd.DataFrame(
        {"ts_ns": (np.arange(n) + 1) * 3_600_000_000_000, "open": close, "high": close, "low": close, "close": close, "volume": 1.0}
    )
    p = tmp_path / "bars.parquet"
    df.to_parquet(p)
    return str(p), df


def payload(tmp_path, code, cuts=(50, 150, 250), **extra):
    path, df = bars_file(tmp_path)
    base = {
        "code": code,
        "params": {"lb": 5},
        "seed": 1,
        "cuts": [int(df.ts_ns[i]) for i in cuts],
        "instrument": "BTC-USD",
        "timeframe": "1h",
        "data": {"primary": path},
    }
    base.update(extra)
    return base


def test_the_composed_source_runs_under_the_runner_namespace_and_returns_the_full_series_and_each_cut(tmp_path):
    out = driver()(payload(tmp_path, CAUSAL))
    d = out["detail"]
    assert len(d["full"]["ts"]) == 300 and len(d["cases"]) == 3
    assert d["cases"][0]["ts"][-1] <= d["cases"][0]["cut_ns"], "a case holds only decisions at or before its cut"
    assert d["position_units"] == "fraction" and d["allow_short"] is False


def test_declared_params_are_reported_for_the_sweep(tmp_path):
    d = driver()(payload(tmp_path, CAUSAL))["detail"]
    assert d["declared_params"] == {
        "lb": {"kind": "int", "low": 2, "high": 50},
        "k": {"kind": "float", "low": 0.5, "high": 3.0},
    }


def test_a_file_that_declares_no_params_is_refused_with_a_stable_code(tmp_path):
    none = CAUSAL.replace('params={"lb": Param.int(2, 50, 5), "k": Param.float(0.5, 3.0, 1.0)}', "params={}")
    out = driver()(payload(tmp_path, none))
    assert out["refused"] and out["reason"] == "meta.no_params"


def test_the_wrong_instrument_or_timeframe_is_refused(tmp_path):
    assert driver()(payload(tmp_path, CAUSAL, instrument="ETH-USD"))["reason"] == "meta.instrument"
    assert driver()(payload(tmp_path, CAUSAL, timeframe="1d"))["reason"] == "meta.timeframe"


def test_a_function_that_never_names_positions_is_refused(tmp_path):
    out = driver()(payload(tmp_path, CAUSAL.replace("def positions", "def other")))
    assert out["refused"] and out["reason"] == "meta.undeclared"


def test_a_look_ahead_function_gives_different_earlier_answers_on_truncated_data(tmp_path):
    leaky = CAUSAL.replace("shift(p.lb)", "shift(-p.lb)")
    d = driver()(payload(tmp_path, leaky))["detail"]
    full = dict(zip(d["full"]["ts"], d["full"]["target"]))
    differs = False
    for c in d["cases"]:
        for t, v in zip(c["ts"], c["target"]):
            if t <= c["cut_ns"] and t in full and full[t] != v and not (full[t] is None and v is None):
                differs = True
    assert differs, "the harness hands Rust series that disagree, which is what the comparison catches"


def test_an_undeclared_param_value_is_refused(tmp_path):
    out = driver()(payload(tmp_path, CAUSAL, params={"lb": 5, "zzz": 1}))
    assert out["refused"] and out["reason"] == "param.undeclared"
