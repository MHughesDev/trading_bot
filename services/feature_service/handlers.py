"""Feature service: market tick → feature row (scaffold or FeaturePipeline parity)."""

from __future__ import annotations

import os

from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.trace import new_trace_id

from services.feature_service.feature_pipeline_integration import FeatureRowBuilder


def _use_feature_pipeline() -> bool:
    return os.getenv("NM_FEATURE_SERVICE_USE_PIPELINE", "true").strip().lower() in (
        "1",
        "true",
        "yes",
    )


class FeatureServiceHandlers:
    """Consumes normalized ticks and emits feature rows for the decision stage."""

    def __init__(self, bus: MessageBus, *, row_builder: FeatureRowBuilder | None = None) -> None:
        self._bus = bus
        self._row_builder = row_builder or FeatureRowBuilder()
        self._use_pipeline = _use_feature_pipeline()

    def on_market_tick(self, env: EventEnvelope) -> None:
        if self._use_pipeline:
            payload = self._row_builder.build_from_tick(env)
            symbol = str(payload.get("symbol", env.symbol or ""))
        else:
            symbol = str(env.payload.get("symbol", env.symbol or ""))
            mid = float(env.payload.get("mid_price", env.payload.get("price", 50_000.0)))
            payload = {
                "symbol": symbol,
                "direction": int(env.payload.get("direction", 1)),
                "size_fraction": float(env.payload.get("size_fraction", 0.1)),
                "route_id": str(env.payload.get("route_id", "SCALPING")),
                "mid_price": mid,
                "spread_bps": float(env.payload.get("spread_bps", 5.0)),
            }
        out = EventEnvelope(
            event_type="features.row.generated",
            event_version="v1",
            trace_id=env.trace_id or new_trace_id(),
            correlation_id=str(env.event_id),
            producer_service="feature_service",
            symbol=symbol or None,
            partition_key=symbol or None,
            payload=payload,
        )
        self._bus.publish(topics.FEATURES_ROW_GENERATED_V1, out)

    def register(self) -> None:
        self._bus.subscribe(topics.MARKET_TICK_NORMALIZED_V1, self.on_market_tick)
