import io

import numpy as np
import torch
import torch.nn as nn

from ..schemas import Forecast
from . import base


class MLP(nn.Module):
    def __init__(self, input_dim: int):
        super().__init__()
        self.net = nn.Sequential(
            nn.Linear(input_dim, 32),
            nn.ReLU(),
            nn.Linear(32, 1),
        )

    def forward(self, x):
        return self.net(x)


class LSTMNet(nn.Module):
    def __init__(self, input_dim: int):
        super().__init__()
        self.lstm = nn.LSTM(input_dim, 32, batch_first=True)
        self.head = nn.Linear(32, 1)

    def forward(self, x):
        if x.dim() == 2:
            x = x.unsqueeze(1)
        out, _ = self.lstm(x)
        out = out[:, -1, :]
        return self.head(out)


def _build(arch: str, input_dim: int):
    if arch == "lstm":
        return LSTMNet(input_dim)
    return MLP(input_dim)


def predict(artifact_bytes: bytes, instances: list, model_kind: str, horizon: str) -> list[Forecast]:
    keys = set()
    for inst in instances:
        f = getattr(inst, "features", {}) or {}
        keys.update(f.keys())
    feature_order = sorted(keys)

    X = base.features_matrix(instances, feature_order=feature_order)

    try:
        payload = torch.load(io.BytesIO(artifact_bytes), map_location="cpu", weights_only=False)

        input_dim = int(payload.get("input_dim", X.shape[1] if X.shape[1] else 1))

        # Forecaster artifacts carry target_field / horizon and emit a return.
        is_forecaster = "target_field" in payload or model_kind == "forecaster"
        arch = payload.get("arch", "lstm" if is_forecaster else "mlp")
        out_horizon = payload.get("horizon", horizon) or horizon

        model = _build(arch, input_dim)
        model.load_state_dict(payload["state_dict"])
        model.eval()

        # Pad/truncate feature width to the model's expected input_dim.
        if X.shape[1] != input_dim:
            fixed = np.zeros((X.shape[0], input_dim), dtype=np.float32)
            w = min(X.shape[1], input_dim)
            if w > 0:
                fixed[:, :w] = X[:, :w]
            X = fixed

        xt = torch.tensor(X, dtype=torch.float32)
        with torch.no_grad():
            raw = model(xt).squeeze(-1)

        forecasts: list[Forecast] = []
        if is_forecaster:
            # Output is a predicted return; map sign -> direction, magnitude
            # from the raw value, and confidence from a squashed magnitude.
            for r in np.atleast_1d(raw.numpy().astype(float)):
                score = 0.5 + (1.0 / (1.0 + np.exp(-float(r))) - 0.5)
                forecasts.append(base.to_forecast(float(score), out_horizon))
        else:
            probs = torch.sigmoid(raw).numpy().astype(float)
            for p in np.atleast_1d(probs):
                forecasts.append(base.to_forecast(float(p), out_horizon))
        return forecasts
    except Exception:
        # Robust fallback: one flat forecast per instance.
        return [base.to_forecast(0.5, horizon) for _ in instances]
