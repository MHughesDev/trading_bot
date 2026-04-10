"""
Forecaster model package (human spec: VSN → Latent CNN → Multi-Resolution xLSTM → …).

Current tree: contracts, config, features, regime, inference stubs.
Full deep learning stack is tracked in `docs/FEATURES_BACKLOG.MD` (FB-FR-*).
"""

from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.stub import build_forecast_packet_stub

__all__ = ["ForecasterConfig", "build_forecast_packet_stub"]
