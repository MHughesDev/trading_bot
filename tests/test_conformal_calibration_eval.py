"""FB-FR-P0-04-03: conformal interval sanity on synthetic residuals."""

from __future__ import annotations

from forecaster_model.calibration.conformal import MultiHorizonConformal


def test_multi_horizon_conformal_widens_interval() -> None:
    m = MultiHorizonConformal.create(3, alpha=0.1, window_size=50)
    for _ in range(20):
        m.update_horizon(0, y_true=0.0, q_low=-0.1, q_high=0.1)
    lo = [-0.1, -0.1, -0.1]
    md = [0.0, 0.0, 0.0]
    hi = [0.1, 0.1, 0.1]
    out_lo, out_md, out_hi = m.apply_to_quantiles(lo, md, hi)
    assert out_lo[0] <= lo[0]
    assert out_hi[0] >= hi[0]
    assert out_md == md
