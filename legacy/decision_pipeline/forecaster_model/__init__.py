"""
Forecaster model package (human spec: VSN → Latent CNN → Multi-Resolution xLSTM → …).

Current tree: contracts, config, features, regime, inference stubs.
Full deep learning stack is tracked in `docs/QUEUE_ARCHIVE.MD` (FB-FR-*).
"""

from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from legacy.decision_pipeline.forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from legacy.decision_pipeline.forecaster_model.inference.stub import build_forecast_packet_stub
from legacy.decision_pipeline.forecaster_model.models.forecaster_model import ForecasterModel

__all__ = [
    "ForecasterConfig",
    "ForecasterModel",
    "build_forecast_packet_stub",
    "build_forecast_packet_methodology",
]
