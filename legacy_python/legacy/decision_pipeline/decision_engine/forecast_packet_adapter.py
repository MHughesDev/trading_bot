"""Map `ForecastPacket` → `ForecastOutput` for routing when packet is the authoritative source (FB-FR-PG1)."""

from __future__ import annotations

from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket


def forecast_packet_to_forecast_output(pkt: ForecastPacket) -> ForecastOutput:
    """
    Derive compact `ForecastOutput` horizons from quantile medians and packet uncertainty.

    Horizons 1/3/5/15 bar steps: use q_med indices when present; shorter horizons
    fall back to the nearest available index (packet may have H < 15).
    """
    med = pkt.q_med
    h = len(med)
    if h == 0:
        return ForecastOutput(
            returns_1=0.0,
            returns_3=0.0,
            returns_5=0.0,
            returns_15=0.0,
            volatility=0.0,
            uncertainty=1.0,
        )

    def _idx(i: int) -> float:
        return float(med[min(i, h - 1)])

    r1 = _idx(0)
    r3 = _idx(2) if h > 2 else r1
    r5 = _idx(4) if h > 4 else float(med[-1])
    r15 = _idx(14) if h > 14 else float(med[-1])

    iv = pkt.interval_width
    vol = float(sum(iv) / len(iv)) if iv else 0.0

    if isinstance(pkt.confidence_score, (int, float)):
        conf = float(pkt.confidence_score)
    else:
        cs = pkt.confidence_score
        conf = float(sum(cs) / max(len(cs), 1)) if cs else 0.0
    unc = float(1.0 / (abs(conf) + 1e-6))

    return ForecastOutput(
        returns_1=r1,
        returns_3=r3,
        returns_5=r5,
        returns_15=r15,
        volatility=max(0.0, vol),
        uncertainty=max(0.0, unc),
    )
