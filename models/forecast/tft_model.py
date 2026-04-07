from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from app.contracts.models import ForecastOutput


@dataclass(slots=True)
class TFTForecastModel:
    """
    V1 forecast model interface.

    This is a lightweight deterministic proxy for a full TFT implementation.
    It preserves the contract needed by downstream routing/risk/execution.
    """

    horizons: tuple[int, ...] = (1, 3, 5, 15)

    def predict(
        self, symbol: str, recent_returns: np.ndarray, recent_volatility: float
    ) -> ForecastOutput:
        if recent_returns.size == 0:
            baseline = 0.0
            vol = max(float(recent_volatility), 0.0)
        else:
            baseline = float(np.clip(np.mean(recent_returns[-5:]), -0.03, 0.03))
            vol = float(
                max(
                    recent_volatility,
                    np.std(recent_returns[-20:]) if recent_returns.size >= 2 else 0.0,
                )
            )

        horizon_returns: dict[int, float] = {}
        for h in self.horizons:
            decay = 1.0 / (1.0 + (h - 1) * 0.35)
            horizon_returns[h] = baseline * decay

        confidence = float(np.clip(1.0 - min(vol / 0.05, 0.85), 0.1, 0.95))
        uncertainty = float(np.clip(vol, 0.0, 1.0))
        return ForecastOutput(
            symbol=symbol,
            horizon_returns=horizon_returns,
            volatility_estimate=vol,
            confidence=confidence,
            uncertainty=uncertainty,
            metadata={"model": "tft_proxy_v1"},
        )
