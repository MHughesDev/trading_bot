import io

import numpy as np
import torch
import torch.nn as nn

from .. import engine


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


def _build_model(arch: str, input_dim: int, hidden: int):
    if arch == "lstm":
        return LSTMNet(input_dim, hidden)
    return MLP(input_dim, hidden)


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    p = engine.prepare(definition, df)
    hp = p.hp

    arch = hp.get("arch", "mlp")
    epochs = int(hp.get("epochs", 50))
    lr = float(hp.get("learning_rate", 1e-3))
    batch_size = int(hp.get("batch_size", 64))
    hidden = int(hp.get("hidden_dim", 32))
    weight_decay = float(hp.get("weight_decay", 0.0))
    input_dim = p.X_tr.shape[1] if p.X_tr.shape[1] else 1

    model = _build_model(arch, input_dim, hidden)
    model, loss_info = engine.train_torch_model(
        model,
        p.X_tr,
        p.y_tr,
        p.X_val,
        p.y_val,
        objective=p.objective,
        epochs=epochs,
        lr=lr,
        batch_size=batch_size,
        emit_progress=emit_progress,
        weight_decay=weight_decay,
    )

    def predict_scores(X):
        if len(X) == 0:
            return np.array([])
        model.eval()
        with torch.no_grad():
            raw = model(torch.tensor(np.asarray(X, dtype=np.float32))).squeeze(-1)
        raw = np.atleast_1d(raw.numpy().astype(float))
        if p.objective == "classification":
            return 1.0 / (1.0 + np.exp(-raw))
        return raw

    if p.objective == "classification":
        parts = [
            engine.classification_metrics("val", p.y_val, predict_scores(p.X_val)),
            engine.classification_metrics("test", p.y_te, predict_scores(p.X_te)),
        ]
    else:
        parts = [
            engine.regression_metrics("val", p.y_val, predict_scores(p.X_val)),
            engine.regression_metrics("test", p.y_te, predict_scores(p.X_te)),
        ]

    metrics = engine.assemble_metrics(
        p.objective,
        parts,
        framework_version=torch.__version__,
        n_train=len(p.X_tr),
        n_val=len(p.X_val),
        n_test=len(p.X_te),
        extra={"arch": arch, "input_dim": int(input_dim), **loss_info},
    )

    buf = io.BytesIO()
    torch.save(
        {
            "state_dict": model.state_dict(),
            "arch": arch,
            "input_dim": int(input_dim),
            "hidden_dim": hidden,
            "objective": p.objective,
        },
        buf,
    )
    bundle = engine.wrap_bundle(
        buf.getvalue(),
        framework="torch",
        objective=p.objective,
        feature_order=p.feature_order,
        scaler=p.scaler,
        arch=arch,
    )
    return bundle, metrics
