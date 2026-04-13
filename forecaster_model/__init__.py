"""
Forecaster model package (human spec: VSN → Latent CNN → Multi-Resolution xLSTM → …).

Current tree: contracts, config, features, regime, inference stubs.
Full deep learning stack is tracked in `docs/QUEUE_ARCHIVE.MD` (FB-FR-*).
"""

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.inference.stub import build_forecast_packet_stub
from forecaster_model.models.forecaster_model import ForecasterModel

__all__ = [
    "ForecasterConfig",
    "ForecasterModel",
    "build_forecast_packet_stub",
    "build_forecast_packet_methodology",
]
