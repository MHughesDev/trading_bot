import io

import numpy as np
import torch
import torch.nn as nn

from ..schemas import Forecast
from . import base


class MLP(nn.Module):
    def __init__(self, input_dim: int, hidden: int = 32):
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(input_dim, hidden),
            nn.ReLU(),
            nn.Linear(hidden, 1),
        )

    def forward(self, x):
        return self.net(x)


class LSTMNet(nn.Module):
    def __init__(self, input_dim: int, hidden: int = 32):
        super().__init__()
        self.lstm = nn.LSTM(input_dim, hidden, batch_first=True)
        self.head = nn.Linear(hidden, 1)

    def forward(self, x):
        if x.dim() == 2:
            x = x.unsqueeze(1)
        out, _ = self.lstm(x)
        out = out[:, -1, :]
        return self.head(out)


def _build(arch: str, input_dim: int, hidden: int = 32):
    if arch == "lstm":
        return LSTMNet(input_dim, hidden)
    return MLP(input_dim, hidden)


def _fit_width(X: np.ndarray, input_dim: int) -> np.ndarray:
    """Pad/truncate the feature width to the model's expected input_dim."""
    if X.shape[1] == input_dim:
        return X
    fixed = np.zeros((X.shape[0], input_dim), dtype=np.float32)
    w = min(X.shape[1], input_dim)
    if w > 0:
        fixed[:, :w] = X[:, :w]
    return fixed


def predict(
    artifact_bytes: bytes,
    instances: list,
    model_kind: str,
    horizon: str,
    header: dict | None = None,
) -> list[Forecast]:
    X, objective = base.build_matrix(instances, header)

    try:
        payload = torch.load(io.BytesIO(artifact_bytes), map_location="cpu", weights_only=False)

        input_dim = int(payload.get("input_dim", X.shape[1] if X.shape[1] else 1))
        hidden = int(payload.get("hidden_dim", 32))

        # Objective: bundle header wins; else fall back to artifact hints.
        is_forecaster = (
            payload.get("forecaster")
            or "target_field" in payload
            or model_kind == "forecaster"
        )
        if header is not None:
            objective = header.get("objective", objective)
        elif is_forecaster or payload.get("objective") == "regression":
            objective = "regression"

        arch = (header or {}).get("arch") or payload.get(
            "arch", "lstm" if objective == "regression" else "mlp"
        )
        out_horizon = payload.get("horizon", horizon) or horizon

        model = _build(arch, input_dim, hidden)
        model.load_state_dict(payload["state_dict"])
        model.eval()

        X = _fit_width(X, input_dim)
        with torch.no_grad():
            raw = model(torch.tensor(X, dtype=torch.float32)).squeeze(-1)
        raw = np.atleast_1d(raw.numpy().astype(float))

        if objective == "regression":
            return [base.to_forecast_return(float(r), out_horizon) for r in raw]
        probs = 1.0 / (1.0 + np.exp(-raw))
        return [base.to_forecast(float(p), out_horizon) for p in probs]
    except Exception:
        # Robust fallback: one flat forecast per instance.
        return [base.to_forecast(0.5, horizon) for _ in instances]
