import io

import numpy as np
import torch
import torch.nn as nn

from .. import engine


class LSTMRegressor(nn.Module):
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


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    """Reference forecaster: an LSTM regressor predicting forward return.

    The direction / magnitude / confidence fields are derived at inference time
    from the predicted scalar return, not produced here.
    """
    engine.seed_everything(definition)
    # Forecasters always regress on the continuous forward return.
    definition = {**definition, "hyperparameters": {**(definition.get("hyperparameters") or {})}}
    definition["hyperparameters"]["objective"] = "regression"
    p = engine.prepare(definition, df)
    hp = p.hp

    target = definition.get("target", {}) or {}
    target_field = target.get("field", "label")
    horizon = target.get("horizon", "1h")

    epochs = int(hp.get("epochs", 50))
    lr = float(hp.get("learning_rate", 1e-3))
    batch_size = int(hp.get("batch_size", 64))
    hidden = int(hp.get("hidden_dim", 32))
    weight_decay = float(hp.get("weight_decay", 0.0))
    input_dim = p.X_tr.shape[1] if p.X_tr.shape[1] else 1

    model = LSTMRegressor(input_dim, hidden)
    model, loss_info = engine.train_torch_model(
        model,
        p.X_tr,
        p.y_tr,
        p.X_val,
        p.y_val,
        objective="regression",
        epochs=epochs,
        lr=lr,
        batch_size=batch_size,
        emit_progress=emit_progress,
        weight_decay=weight_decay,
    )

    def predict(X):
        if len(X) == 0:
            return np.array([])
        model.eval()
        with torch.no_grad():
            raw = model(torch.tensor(np.asarray(X, dtype=np.float32))).squeeze(-1)
        return np.atleast_1d(raw.numpy().astype(float))

    parts = [
        engine.regression_metrics("val", p.y_val, predict(p.X_val)),
        engine.regression_metrics("test", p.y_te, predict(p.X_te)),
    ]
    metrics = engine.assemble_metrics(
        "regression",
        parts,
        framework_version=torch.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={"arch": "lstm", "horizon": horizon, "input_dim": int(input_dim), **loss_info},
    )

    buf = io.BytesIO()
    torch.save(
        {
            "state_dict": model.state_dict(),
            "arch": "lstm",
            "input_dim": int(input_dim),
            "hidden_dim": hidden,
            "objective": "regression",
            "horizon": horizon,
            "target_field": target_field,
        },
        buf,
    )
    bundle = engine.wrap_bundle(
        buf.getvalue(),
        framework="torch",
        objective="regression",
        feature_order=p.feature_order,
        scaler=p.scaler,
        arch="lstm",
        extra={"horizon": horizon, "target_field": target_field, "forecaster": True},
    )
    return bundle, metrics
