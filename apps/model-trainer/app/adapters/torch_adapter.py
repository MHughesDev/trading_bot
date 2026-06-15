import io

import numpy as np
import torch
import torch.nn as nn

from .base import split_label, train_val_split


def _binarize(y):
    nunique = y.nunique(dropna=True)
    if nunique > 2:
        y = (y > y.median()).astype(int)
    else:
        y = y.astype(int)
    return y


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
        # x: (batch, input_dim) -> treat as sequence of length 1
        if x.dim() == 2:
            x = x.unsqueeze(1)
        out, _ = self.lstm(x)
        out = out[:, -1, :]
        return self.head(out)


def _build_model(arch: str, input_dim: int):
    if arch == "lstm":
        return LSTMNet(input_dim)
    return MLP(input_dim)


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    hp = definition.get("hyperparameters", {}) or {}
    arch = hp.get("arch", "mlp")
    epochs = int(hp.get("epochs", 20))
    lr = float(hp.get("learning_rate", 1e-3))

    X, y = split_label(df)
    y = _binarize(y)

    X_tr, y_tr, X_val, y_val = train_val_split(X, y, frac=0.8)
    input_dim = X.shape[1]

    Xtr_t = torch.tensor(X_tr.to_numpy(dtype=np.float32))
    ytr_t = torch.tensor(y_tr.to_numpy(dtype=np.float32)).unsqueeze(1)
    Xval_t = torch.tensor(X_val.to_numpy(dtype=np.float32))
    yval_t = torch.tensor(y_val.to_numpy(dtype=np.float32)).unsqueeze(1)

    model = _build_model(arch, input_dim)
    optimizer = torch.optim.Adam(model.parameters(), lr=lr)
    loss_fn = nn.BCEWithLogitsLoss()

    train_loss = float("nan")
    val_loss = float("nan")

    for epoch in range(epochs):
        model.train()
        optimizer.zero_grad()
        out = model(Xtr_t)
        loss = loss_fn(out, ytr_t)
        loss.backward()
        optimizer.step()
        train_loss = float(loss.item())

        model.eval()
        with torch.no_grad():
            if len(Xval_t) > 0:
                vout = model(Xval_t)
                val_loss = float(loss_fn(vout, yval_t).item())

        pct = ((epoch + 1) / max(epochs, 1)) * 100.0
        emit_progress("fitting", pct, {"epoch": epoch + 1, "train_loss": train_loss, "val_loss": val_loss})

    buf = io.BytesIO()
    torch.save(
        {"state_dict": model.state_dict(), "arch": arch, "input_dim": input_dim},
        buf,
    )
    artifact_bytes = buf.getvalue()

    metrics = {
        "train_loss": train_loss,
        "val_loss": val_loss,
        "n_train": int(len(X_tr)),
        "n_val": int(len(X_val)),
        "arch": arch,
        "input_dim": int(input_dim),
        "framework_version": torch.__version__,
    }
    return artifact_bytes, metrics
