from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.inference.guards import ForecastGuard, ForecastGuardConfig


def test_drift_abstain():
    pkt = ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1],
        q_low=[-0.01],
        q_med=[10.0],
        q_high=[0.01],
        interval_width=[0.02],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=1.0,
        ensemble_variance=[1e-12],
        ood_score=0.0,
    )
    g = ForecastGuard(
        ForecastGuardConfig(
            max_interval_width=100.0,
            max_ensemble_variance=1.0,
            drift_reference_abs_qmed_mean=0.0,
            drift_max_zscore=1.0,
            drift_reference_std=0.01,
        )
    )
    ok, reasons = g.check(pkt)
    assert not ok
    assert "feature_drift" in reasons
