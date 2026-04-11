"""Feature service: RollingBars + FeaturePipeline integration."""

from __future__ import annotations

from datetime import UTC, datetime

from shared.messaging.envelope import EventEnvelope
from services.feature_service.feature_pipeline_integration import FeatureRowBuilder


def test_feature_row_builder_includes_schema_version() -> None:
    b = FeatureRowBuilder()
    env = EventEnvelope(
        event_type="market.tick.normalized",
        trace_id="t1",
        producer_service="market_data_service",
        symbol="BTC-USD",
        payload={
            "symbol": "BTC-USD",
            "mid_price": 50_000.0,
            "direction": 1,
            "size_fraction": 0.1,
            "route_id": "SCALPING",
            "spread_bps": 10.0,
            "data_timestamp": datetime(2026, 1, 1, 0, 0, 0, tzinfo=UTC).isoformat(),
        },
    )
    row = b.build_from_tick(env)
    assert row["symbol"] == "BTC-USD"
    assert "feature_schema_version" in row
    assert "rsi_14" in row or "macd" in row
