"""Project :class:`ForecastPacket` → :class:`CanonicalStructureOutput` (FB-CAN-017)."""

from __future__ import annotations

import statistics

from app.contracts.canonical_structure import CanonicalStructureOutput
from app.contracts.forecast_packet import ForecastPacket


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def structure_from_forecast_packet(pkt: ForecastPacket) -> CanonicalStructureOutput:
    """
    Derive canonical structure fields from the legacy probabilistic packet.

    Quantiles: horizon-0 primary; linear blend for inner percentiles when H>=2.
    """
    h = len(pkt.horizons)
    if h == 0:
        return CanonicalStructureOutput(
            p05=0.0,
            p25=0.0,
            p50=0.0,
            p75=0.0,
            p95=0.0,
            volatility_forecast=0.0,
            asymmetry_score=0.0,
            continuation_probability=0.0,
            fragility_score=1.0,
            directional_bias=0.0,
            model_agreement_score=0.0,
            model_correlation_penalty=0.5,
            calibration_weight=0.0,
        )

    lo0 = float(pkt.q_low[0])
    hi0 = float(pkt.q_high[0])
    med0 = float(pkt.q_med[0])
    width0 = max(hi0 - lo0, 1e-12)

    p05 = lo0
    p95 = hi0
    p50 = med0
    if h >= 2:
        lo1 = float(pkt.q_low[1])
        hi1 = float(pkt.q_high[1])
        p25 = 0.75 * lo0 + 0.25 * med0
        p75 = 0.25 * med0 + 0.75 * hi0
        # slight cross-horizon tension increases fragility proxy
        tension = abs((hi1 - lo1) - width0) / (width0 + 1e-9)
    else:
        p25 = 0.5 * lo0 + 0.5 * med0
        p75 = 0.5 * med0 + 0.5 * hi0
        tension = 0.0

    iv = pkt.interval_width
    vol_f = float(sum(iv) / len(iv)) if iv else 0.0

    asym = abs(((med0 - lo0) / width0) - 0.5) * 2.0
    asym = _clip01(asym)

    if isinstance(pkt.confidence_score, (int, float)):
        conf = float(pkt.confidence_score)
    else:
        cs = pkt.confidence_score
        conf = float(sum(cs) / max(len(cs), 1)) if cs else 0.0
    conf01 = _clip01(conf)

    ood = _clip01(float(pkt.ood_score))
    frag = _clip01(0.55 * ood + 0.25 * _clip01(vol_f * 5.0) + 0.2 * _clip01(tension))

    skew = ((med0 - lo0) / width0 - 0.5) * 2.0
    directional_bias = max(-1.0, min(1.0, float(skew)))

    ens = pkt.ensemble_variance
    if ens and len(ens) > 1:
        spread = float(statistics.pstdev(ens)) if len(ens) > 1 else 0.0
        mcp = _clip01(spread / (abs(ens[0]) + 1e-9))
    else:
        mcp = 0.0

    cont = _clip01(0.5 * conf01 + 0.5 * (1.0 - frag))

    oi_class = "unknown"
    fd = pkt.forecast_diagnostics or {}
    oc = fd.get("oi_structure_class")
    if isinstance(oc, str) and oc:
        oi_class = oc

    return CanonicalStructureOutput(
        p05=p05,
        p25=p25,
        p50=p50,
        p75=p75,
        p95=p95,
        volatility_forecast=max(0.0, vol_f),
        asymmetry_score=asym,
        continuation_probability=cont,
        fragility_score=frag,
        directional_bias=directional_bias,
        model_agreement_score=conf01,
        model_correlation_penalty=mcp,
        calibration_weight=_clip01(
            float(fd["calibration_weight"]) if "calibration_weight" in fd else 1.0
        ),
        oi_structure_class=oi_class,
    )
