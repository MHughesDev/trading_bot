"""Hard override taxonomy for canonical safety / degradation (FB-CAN-033).

See APEX_State_Regime_Logic_Detail_Spec_v1_0.md §11.2 (hard_override branch) and monitoring spec.
"""

from __future__ import annotations

from enum import StrEnum


class HardOverrideKind(StrEnum):
    """Explicit override reason; ``none`` means soft degradation rules only."""

    NONE = "none"
    SYSTEM_MODE = "system_mode"
    FEED_STALE = "feed_stale"
    DATA_TIMESTAMP_STALE = "data_timestamp_stale"
    SPREAD_WIDE = "spread_wide"
    DRAWDOWN = "drawdown"
    PRODUCT_UNTRADABLE = "product_untradable"
    NORMALIZATION_INCOMPLETE = "normalization_incomplete"
    SIGNAL_CONFIDENCE_LOW = "signal_confidence_low"
    # FB-CAN-074 — exchange risk / data integrity (APEX State spec §13)
    DATA_INTEGRITY_ALERT = "data_integrity_alert"
    EXCHANGE_RISK_CRITICAL = "exchange_risk_critical"


__all__ = ["HardOverrideKind"]
