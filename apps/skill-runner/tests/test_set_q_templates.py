"""Acceptance clauses of the Set Q templates (Q-1.2 to Q-1.5), run the way the runner runs them:
the preamble is prepended to the template body and `run(payload)` is called.

The platform-side suite `crates/mcp-server/tests/templates_run.rs` additionally runs every
template twice on the reference dataset and compares `detail` byte for byte; this file holds
what that one cannot, the planted-answer cases.
"""

import json
import os

import numpy as np
import pandas as pd
import pytest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
PREAMBLE = open(os.path.join(ROOT, "apps", "skill-runner", "preamble.py"), encoding="utf-8").read()


def go(rel, payload):
    src = open(os.path.join(ROOT, "apps", "skill-runner", "templates", rel), encoding="utf-8").read()
    ns = {"__name__": "tpl"}
    exec(compile(PREAMBLE + "\n" + src, rel, "exec"), ns)
    return ns["run"](payload)


def bars_file(tmp_path, close, volume=None, name="bars.parquet"):
    n = len(close)
    idx = pd.date_range("2024-01-01", periods=n, freq="1h", tz="UTC")
    frame = pd.DataFrame(
        {
            "open": close,
            "high": np.asarray(close) * 1.001,
            "low": np.asarray(close) * 0.999,
            "close": close,
            "volume": np.full(n, 100.0) if volume is None else volume,
        },
        index=idx,
    )
    frame.index.name = "ts"
    p = tmp_path / name
    frame.reset_index().to_parquet(p)
    return str(p)


def family_file(tmp_path, columns, name="family.parquet"):
    rows = []
    for rid, series in columns.items():
        for i, r in enumerate(series):
            rows.append({"run_id": rid, "period": i, "ret": float(r)})
    p = tmp_path / name
    pd.DataFrame(rows).to_parquet(p)
    return str(p)


# ── posterior_alpha ─────────────────────────────────────────────────────────


def test_a_family_with_one_planted_winner_is_pulled_back_toward_the_theme(tmp_path):
    r = np.random.default_rng(1)
    n = 400
    cols = {f"r{i:02d}": r.normal(0.0, 0.01, n) for i in range(12)}
    cols["r05"] = cols["r05"] + 0.0012  # the winner: a real but modest edge
    p = family_file(tmp_path, cols)
    out = go("attribution/posterior_alpha.py", {"experiment_id": "e", "seed": 7, "data": {"family": p}})
    d = out["detail"]
    assert d["candidate"] == "r05" or d["raw_mean"] == max(m["raw_mean"] for m in d["members"])
    assert 0.0 < d["shrinkage"] <= 1.0
    # Shrinkage moves the estimate toward the theme mean and never past it.
    lo, hi = sorted([d["prior_mean"], d["raw_mean"]])
    assert lo - 1e-12 <= d["post_mean"] <= hi + 1e-12
    assert abs(d["post_mean"] - d["prior_mean"]) <= abs(d["raw_mean"] - d["prior_mean"])
    assert d["prior_source"] == "family"
    assert "winner's curse" in d["selected_by"]


def test_a_family_with_a_common_mean_collapses_onto_it_and_says_so(tmp_path):
    r = np.random.default_rng(2)
    # Members whose sample means are identical by construction: no spread between candidates.
    cols = {}
    for i in range(10):
        x = r.normal(0.0, 0.01, 500)
        cols[f"r{i:02d}"] = x - x.mean() + 0.0005
    p = family_file(tmp_path, cols)
    d = go("attribution/posterior_alpha.py", {"experiment_id": "e", "seed": 7, "data": {"family": p}})["detail"]
    assert d["between_sd"] == 0.0
    assert "no between-candidate spread" in d["verdict"]
    assert d["shrinkage"] == 1.0
    # The interval is not degenerate: the uncertainty in the family mean is carried.
    assert d["ci_hi"] > d["ci_lo"]


def test_a_supplied_prior_is_labelled_and_needs_its_spread(tmp_path):
    r = np.random.default_rng(3)
    cols = {f"r{i:02d}": r.normal(0.0, 0.01, 300) for i in range(5)}
    p = family_file(tmp_path, cols)
    base = {"experiment_id": "e", "seed": 7, "data": {"family": p}}
    refused = go("attribution/posterior_alpha.py", dict(base, prior_mean=0.0))
    assert refused.get("refused")
    ok = go("attribution/posterior_alpha.py", dict(base, prior_mean=0.0, prior_sd=0.0005))
    assert ok["detail"]["prior_source"] == "supplied"


def test_two_members_are_not_a_family_and_an_unknown_candidate_is_refused(tmp_path):
    r = np.random.default_rng(4)
    two = family_file(tmp_path, {"a": r.normal(0, 0.01, 200), "b": r.normal(0, 0.01, 200)})
    assert go("attribution/posterior_alpha.py", {"experiment_id": "e", "data": {"family": two}}).get("refused")
    five = family_file(tmp_path, {f"r{i}": r.normal(0, 0.01, 200) for i in range(5)}, "five.parquet")
    out = go("attribution/posterior_alpha.py", {"experiment_id": "e", "candidate": "zzz", "data": {"family": five}})
    assert out.get("refused") and "members" in out["detail"]


def test_posterior_alpha_is_reproducible(tmp_path):
    r = np.random.default_rng(5)
    p = family_file(tmp_path, {f"r{i}": r.normal(0, 0.01, 200) for i in range(6)})
    a = go("attribution/posterior_alpha.py", {"experiment_id": "e", "data": {"family": p}})
    b = go("attribution/posterior_alpha.py", {"experiment_id": "e", "data": {"family": p}})
    assert json.dumps(a["detail"], sort_keys=True) == json.dumps(b["detail"], sort_keys=True)


# ── capacity_report ─────────────────────────────────────────────────────────


def hourly_bars(tmp_path, days=40, vol=0.01, volume=100.0, name="cap.parquet"):
    r = np.random.default_rng(11)
    n = days * 24
    # Daily vol `vol` => hourly vol vol / sqrt(24).
    close = 100.0 * np.exp(np.cumsum(r.normal(0, vol / np.sqrt(24), n)))
    return bars_file(tmp_path, close, np.full(n, volume), name)


CAP = {"instrument_id": "X", "timeframe": "1h", "start": "a", "end": "b", "seed": 1}


def test_net_edge_falls_with_size_and_break_even_scales_with_volume(tmp_path):
    lo = go("portfolio/capacity_report.py", dict(CAP, gross_edge_bps=80, data={"primary": hourly_bars(tmp_path, volume=100.0, name="a.parquet")}))["detail"]
    hi = go("portfolio/capacity_report.py", dict(CAP, gross_edge_bps=80, data={"primary": hourly_bars(tmp_path, volume=400.0, name="b.parquet")}))["detail"]
    nets = [row["net_edge_bps"] for row in lo["curve"]]
    assert all(a > b for a, b in zip(nets, nets[1:])), "net edge is strictly decreasing in size"
    assert lo["break_even_usd"] > 0
    # Four times the volume, four times the ADV: break-even size scales with ADV (same sigma).
    assert hi["adv_usd"] == pytest.approx(4 * lo["adv_usd"], rel=0.01)
    assert hi["break_even_usd"] == pytest.approx(4 * lo["break_even_usd"], rel=0.05)


def test_break_even_falls_with_volatility(tmp_path):
    calm = go("portfolio/capacity_report.py", dict(CAP, gross_edge_bps=80, data={"primary": hourly_bars(tmp_path, vol=0.005, name="c.parquet")}))["detail"]
    wild = go("portfolio/capacity_report.py", dict(CAP, gross_edge_bps=80, data={"primary": hourly_bars(tmp_path, vol=0.03, name="w.parquet")}))["detail"]
    assert wild["break_even_pct_adv"] < calm["break_even_pct_adv"]


def test_an_edge_below_the_fixed_cost_has_a_zero_break_even_and_says_why(tmp_path):
    p = hourly_bars(tmp_path)
    d = go("portfolio/capacity_report.py", dict(CAP, gross_edge_bps=20, data={"primary": p}))["detail"]
    assert d["break_even_usd"] == 0.0  # 2 * (10 + 2) = 24 bps fixed
    assert "does not cover" in d["verdict"]


def test_no_edge_declared_returns_the_cost_curve_and_no_break_even(tmp_path):
    d = go("portfolio/capacity_report.py", dict(CAP, data={"primary": hourly_bars(tmp_path)}))["detail"]
    assert d["break_even_usd"] is None and d["gross_edge_bps"] is None
    assert d["curve"] and "net_edge_bps" not in d["curve"][0]
    assert d["assumptions"]["spread_source"].startswith("placeholder")
    assert d["assumptions"]["impact_y_source"].startswith("spec default")


def test_declared_costs_are_labelled_declared_and_raise_the_cost(tmp_path):
    p = hourly_bars(tmp_path)
    a = go("portfolio/capacity_report.py", dict(CAP, data={"primary": p}))["detail"]
    b = go("portfolio/capacity_report.py", dict(CAP, fee_bps=30, data={"primary": p}))["detail"]
    assert b["assumptions"]["fee_source"] == "declared"
    assert b["cost_at_1pct_bps"] == pytest.approx(a["cost_at_1pct_bps"] + 40.0)


def test_too_few_bars_are_refused(tmp_path):
    p = bars_file(tmp_path, np.linspace(100, 101, 20), name="few.parquet")
    assert go("portfolio/capacity_report.py", dict(CAP, data={"primary": p})).get("refused")


# ── benchmark_run ───────────────────────────────────────────────────────────

BM = {"instrument_id": "X", "timeframe": "1h", "start": "a", "end": "b", "seed": 7}


def test_buy_and_hold_is_the_price_return_net_of_one_round_trip(tmp_path):
    close = np.linspace(100.0, 150.0, 400)
    p = bars_file(tmp_path, close)
    d = go("attribution/benchmark_run.py", dict(BM, n_trades=5, fee_bps=10, half_spread_bps=0, data={"primary": p}))["detail"]
    assert d["buy_hold_return_pct"] == pytest.approx((150.0 / 100.0 - 1.0 - 2 * 0.001) * 100.0)


def test_costs_lower_every_baseline(tmp_path):
    r = np.random.default_rng(9)
    close = 100 * np.exp(np.cumsum(r.normal(0.0002, 0.005, 800)))
    p = bars_file(tmp_path, close)
    free = go("attribution/benchmark_run.py", dict(BM, fee_bps=0, half_spread_bps=0, data={"primary": p}))["detail"]
    paid = go("attribution/benchmark_run.py", dict(BM, fee_bps=20, half_spread_bps=5, data={"primary": p}))["detail"]
    for k in ("buy_hold_return_pct", "random_median_pct", "momentum_return_pct"):
        assert paid[k] < free[k], k


def test_the_random_baseline_is_a_seeded_distribution(tmp_path):
    r = np.random.default_rng(10)
    p = bars_file(tmp_path, 100 * np.exp(np.cumsum(r.normal(0, 0.005, 800))))
    a = go("attribution/benchmark_run.py", dict(BM, data={"primary": p}))["detail"]
    b = go("attribution/benchmark_run.py", dict(BM, data={"primary": p}))["detail"]
    c = go("attribution/benchmark_run.py", dict(BM, seed=8, data={"primary": p}))["detail"]
    assert a["random_quantiles_pct"] == b["random_quantiles_pct"]
    assert a["random_quantiles_pct"] != c["random_quantiles_pct"]
    q = a["random_quantiles_pct"]
    assert q["5"] <= q["25"] <= q["50"] <= q["75"] <= q["95"]
    assert a["hurdle_pct"] == pytest.approx(max(a["buy_hold_return_pct"], a["random_p95_pct"], a["momentum_return_pct"]))


def test_momentum_never_trades_on_the_bar_it_earns(tmp_path):
    # A series that jumps +20% on its last bar only. A rule that peeked at the bar it
    # earns would be long into it; the lagged rule must not capture that jump.
    close = np.concatenate([np.linspace(100, 99, 99), [99.0 * 1.2]])
    p = bars_file(tmp_path, close, name="jump.parquet")
    d = go("attribution/benchmark_run.py", dict(BM, fee_bps=0, half_spread_bps=0, momentum_lookback=10, n_trades=1, hold_bars=10, data={"primary": p}))["detail"]
    assert d["momentum_return_pct"] < 1.0, "the final jump was not available to a lagged rule"
    assert d["buy_hold_return_pct"] > 15.0


def test_an_unholdable_trade_count_is_a_refusal_not_an_overlap(tmp_path):
    p = bars_file(tmp_path, np.linspace(100, 110, 100), name="short.parquet")
    out = go("attribution/benchmark_run.py", dict(BM, n_trades=50, hold_bars=24, data={"primary": p}))
    assert out.get("refused") and out["detail"]["slots"] == 4


# ── expected_decay ──────────────────────────────────────────────────────────


def test_a_published_idea_gets_the_larger_haircut():
    pub = go("attribution/expected_decay.py", {"source": "published", "in_sample_metric": 2.0})["detail"]
    house = go("attribution/expected_decay.py", {"source": "in_house", "in_sample_metric": 2.0})["detail"]
    unknown = go("attribution/expected_decay.py", {})["detail"]
    assert pub["applied_haircut_pct"] == 58.0 and house["applied_haircut_pct"] == 26.0
    assert unknown["applied_haircut_pct"] == 26.0 and unknown["source"] == "unknown"
    assert pub["metric_after_haircut"] == pytest.approx(2.0 * 0.42)
    assert house["metric_after_haircut"] == pytest.approx(2.0 * 0.74)
    assert unknown["metric_after_haircut"] is None
    assert "McLean" in pub["citation"]
    assert any("prior" in w for w in pub["warnings"])


def test_an_unknown_source_is_refused():
    assert go("attribution/expected_decay.py", {"source": "tweet"}).get("refused")


# ── decompose_strategy_returns (Q-1.10) ─────────────────────────────────────

DEC = {"instrument_id": "X", "timeframe": "1h", "start": "a", "end": "b", "seed": 1}


def regime_close(n=1500, seed=21):
    """Alternating calm (rising) and wild (falling) regimes, so volatility and return are related."""
    r = np.random.default_rng(seed)
    rets = np.empty(n - 1)
    for k in range(n - 1):
        calm = (k // 100) % 2 == 0
        rets[k] = r.normal(0.0004, 0.002) if calm else r.normal(-0.0004, 0.012)
    return 100.0 * np.concatenate([[1.0], np.cumprod(1.0 + rets)])


def test_a_constant_long_exposure_is_all_drift(tmp_path):
    close = regime_close()
    p = bars_file(tmp_path, close)
    d = go("attribution/decompose_strategy_returns.py", dict(DEC, exposure=[1.0] * len(close), data={"primary": p}))["detail"]
    assert d["timing_pct"] == pytest.approx(0.0, abs=1e-9)
    assert d["selection_pct"] == pytest.approx(0.0, abs=1e-9)
    assert d["drift_pct"] == pytest.approx(d["total_pct"], abs=1e-9)
    assert "constant exposure" in d["reading"]


def test_a_vol_timed_long_shows_a_timing_component(tmp_path):
    close = regime_close()
    p = bars_file(tmp_path, close)
    d = go("attribution/decompose_strategy_returns.py", dict(DEC, reference_rule="vol_target", data={"primary": p}))["detail"]
    assert d["exposure_source"] == "reference:vol_target"
    assert d["exposure_vol_r2"] > 0.8
    # Volatility is high exactly when the asset falls, so scaling down in it earns a positive timing term.
    assert d["timing_pct"] > 0.5
    assert abs(d["selection_pct"]) < abs(d["timing_pct"])


def test_the_components_sum_to_the_total_within_rounding(tmp_path):
    close = regime_close(seed=33)
    r = np.random.default_rng(5)
    p = bars_file(tmp_path, close)
    exposure = list(r.uniform(-1, 1, len(close)))  # a signal that has nothing to do with volatility
    d = go("attribution/decompose_strategy_returns.py", dict(DEC, exposure=exposure, data={"primary": p}))["detail"]
    assert abs(d["components_minus_total"]) < 1e-9
    assert d["drift_pct"] + d["timing_pct"] + d["selection_pct"] == pytest.approx(d["total_pct"], abs=1e-6)


def test_an_exposure_of_the_wrong_length_is_refused(tmp_path):
    p = bars_file(tmp_path, regime_close())
    out = go("attribution/decompose_strategy_returns.py", dict(DEC, exposure=[1.0] * 50, data={"primary": p}))
    assert out.get("refused") and "one value per bar" in out["reason"]


# ── collider_check (Q-1.12) ─────────────────────────────────────────────────

SIGNAL = ["signal_score", "ma_slope"]
FORWARD = ["post_entry_vol", "forward_return"]


def collide(filters, signal=SIGNAL, forward=FORWARD):
    return go("audit/collider_check.py", {"filters": filters, "signal_quantities": signal, "forward_quantities": forward})


def test_a_filter_of_the_signal_and_a_forward_quantity_is_flagged(tmp_path):
    out = collide([{"name": "calm_after_entry", "depends_on": ["post_entry_vol", "signal_score"]}])["detail"]
    assert out["n_flagged"] == 1 and out["filters"][0]["verdict"] == "possible_collider"
    assert out["check_type"] == "structural check"
    assert "never proves" in out["warnings"][0]


def test_a_declared_confounder_is_not_flagged(tmp_path):
    out = collide([{"name": "trend", "depends_on": ["ma_slope"], "causal_role": "confounder"}])["detail"]
    assert out["n_flagged"] == 0 and out["filters"][0]["verdict"] == "declared_confounder"
    assert "not verified" in out["filters"][0]["note"]


def test_a_forward_only_filter_is_a_lookahead_fault_not_a_collider(tmp_path):
    out = collide([{"name": "later_vol", "depends_on": ["post_entry_vol"]}])["detail"]
    assert out["filters"][0]["verdict"] == "forward_looking" and out["n_flagged"] == 1


def test_a_filter_with_no_declared_overlap_passes_and_empty_declarations_refuse(tmp_path):
    ok = collide([{"name": "weekday", "depends_on": ["day_of_week"]}])["detail"]
    assert ok["n_flagged"] == 0 and ok["filters"][0]["verdict"] == "unrelated"
    assert go("audit/collider_check.py", {"filters": [{"name": "f", "depends_on": ["x"]}], "signal_quantities": [], "forward_quantities": []}).get("refused")


# ── allocate_book names its covariance estimator (Q-2.4, T9) ────────────────


def test_allocate_book_defaults_to_the_shrunk_covariance_and_names_it(tmp_path):
    r = np.random.default_rng(31)
    cols = {f"r{i}": r.normal(0, 0.01, 200) for i in range(6)}
    p = family_file(tmp_path, cols)
    d = go("portfolio/allocate_book.py", {"experiment_id": "e", "objective": "minimum_variance", "data": {"family": p}})["detail"]
    assert d["covariance_estimator"] == "ledoit_wolf_constant_correlation"
    assert 0.0 <= d["shrinkage_intensity"] <= 1.0


def test_the_sample_covariance_is_reachable_only_by_name_and_says_so(tmp_path):
    r = np.random.default_rng(32)
    p = family_file(tmp_path, {f"r{i}": r.normal(0, 0.01, 200) for i in range(6)})
    base = {"experiment_id": "e", "objective": "minimum_variance", "data": {"family": p}}
    default = go("portfolio/allocate_book.py", base)["detail"]
    sample = go("portfolio/allocate_book.py", dict(base, covariance_estimator="sample"))["detail"]
    assert sample["covariance_estimator"].startswith("sample (explicitly requested)")
    assert any("asked for by name" in w for w in sample["warnings"])
    assert not any("asked for by name" in w for w in default["warnings"])
    assert default["weights"] != sample["weights"], "the estimator changes the minimum-variance book"
    assert go("portfolio/allocate_book.py", dict(base, covariance_estimator="bogus")).get("refused")
