"""The sequence family (Set N, N-2.5 and N-2.6; FEAT-006 section 4).

LSTM, TCN, DLinear, N-HiTS, PatchTST, TFT and TSMixer, each a compact
implementation that consumes real windows `(batch, window, features)`; see
`app/windows.py` for how rows become windows and for the serving side.

Every variant:

  * requires `definition["declaration"]["sequence"]` (refuses without it: a
    sequence model on 2-D rows is the seq-len-1 bug this family exists to fix);
  * requires `definition["max_gpu_hours"]` and stops at that wall-clock budget,
    checked once per epoch (it records `budget_exhausted` and the epochs run);
  * records a comparison against a GBM on the same windows flattened into lagged
    features (`vs_gbm`), and carries `promotable: false` unless that comparison
    was recorded and the sequence model beats the GBM (FEAT-006 section 4: a
    sequence model "must beat GBM on lagged features to be used downstream").

The models are small on purpose: the zoo is a capability bounded by the Pascal
GPU and `max_gpu_hours` (ADR-N-02), and the tests train them on CPU in seconds.
"""

from __future__ import annotations

import io
import math
import time

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

from .. import engine, failures, windows


# --------------------------------------------------------------------------- #
# Architectures. Every module maps (B, W, F) -> (B, 1).
# --------------------------------------------------------------------------- #
class LSTMNet(nn.Module):
    def __init__(self, input_dim: int, hidden: int = 16, window: int = 0):
        super().__init__()
        self.lstm = nn.LSTM(input_dim, hidden, batch_first=True)
        self.head = nn.Linear(hidden, 1)

    def forward(self, x):
        out, _ = self.lstm(x)
        return self.head(out[:, -1, :])


class TCNNet(nn.Module):
    """Causal dilated convolutions with residual connections."""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 0, kernel: int = 3, levels: int = 3):
        super().__init__()
        self.inp = nn.Conv1d(input_dim, hidden, 1)
        self.kernel = kernel
        self.dil = [2**i for i in range(levels)]
        self.convs = nn.ModuleList(nn.Conv1d(hidden, hidden, kernel, dilation=d) for d in self.dil)
        self.head = nn.Linear(hidden, 1)

    def forward(self, x):
        h = self.inp(x.transpose(1, 2))
        for conv, d in zip(self.convs, self.dil):
            pad = (self.kernel - 1) * d
            h = h + F.gelu(conv(F.pad(h, (pad, 0))))  # left pad only: causal
        return self.head(h[:, :, -1])


class DLinearNet(nn.Module):
    """Moving-average trend + remainder, one linear map over time each."""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 16, kernel: int = 5):
        super().__init__()
        self.kernel = max(1, min(kernel, window))
        self.trend = nn.Linear(window, 1)
        self.season = nn.Linear(window, 1)
        self.head = nn.Linear(input_dim, 1)

    def forward(self, x):
        xt = x.transpose(1, 2)  # (B, F, W)
        k = self.kernel
        padded = F.pad(xt, (k - 1, 0), mode="replicate")  # causal moving average
        trend = F.avg_pool1d(padded, k, stride=1)
        season = xt - trend
        z = self.trend(trend).squeeze(-1) + self.season(season).squeeze(-1)  # (B, F)
        return self.head(z)


class NHiTSNet(nn.Module):
    """Compact N-HiTS: stacked blocks, each on a max-pooled (multi-rate) view of
    the residual, each emitting a backcast to subtract and a forecast to add."""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 16, pools=(1, 2, 4)):
        super().__init__()
        self.pools = [max(1, min(p, window)) for p in pools]
        self.out_len = [max(1, window // p) for p in self.pools]
        self.input_dim = input_dim
        self.window = window
        self.blocks = nn.ModuleList()
        for ol in self.out_len:
            self.blocks.append(
                nn.ModuleDict(
                    {
                        "mlp": nn.Sequential(nn.Linear(ol * input_dim, hidden), nn.ReLU(), nn.Linear(hidden, hidden), nn.ReLU()),
                        "back": nn.Linear(hidden, window * input_dim),
                        "fore": nn.Linear(hidden, 1),
                    }
                )
            )

    def forward(self, x):
        resid = x
        total = 0.0
        for blk, ol in zip(self.blocks, self.out_len):
            pooled = F.adaptive_max_pool1d(resid.transpose(1, 2), ol).transpose(1, 2)  # (B, ol, F)
            h = blk["mlp"](pooled.reshape(len(x), -1))
            resid = resid - blk["back"](h).reshape(-1, self.window, self.input_dim)
            total = total + blk["fore"](h)
        return total


class PatchTSTNet(nn.Module):
    """Compact PatchTST: channel-independent patches into a small Transformer."""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 16, patch: int = 0):
        super().__init__()
        self.patch = patch or max(2, window // 4)
        self.n_patches = max(1, math.ceil(window / self.patch))
        self.window = window
        self.embed = nn.Linear(self.patch, hidden)
        self.pos = nn.Parameter(torch.zeros(1, self.n_patches, hidden))
        layer = nn.TransformerEncoderLayer(hidden, nhead=2, dim_feedforward=hidden * 2, dropout=0.0, batch_first=True)
        self.enc = nn.TransformerEncoder(layer, num_layers=1, enable_nested_tensor=False)
        self.proj = nn.Linear(self.n_patches * hidden, 1)
        self.head = nn.Linear(input_dim, 1)

    def forward(self, x):
        B, W, Fdim = x.shape
        pad = self.n_patches * self.patch - W
        xt = F.pad(x.transpose(1, 2), (pad, 0))  # (B, F, W+pad): pad the OLD end
        patches = xt.reshape(B * Fdim, self.n_patches, self.patch)
        h = self.enc(self.embed(patches) + self.pos)
        z = self.proj(h.reshape(B * Fdim, -1)).reshape(B, Fdim)
        return self.head(z)


class _GRN(nn.Module):
    """Gated residual network (TFT building block)."""

    def __init__(self, d_in: int, d_hidden: int, d_out: int):
        super().__init__()
        self.fc1 = nn.Linear(d_in, d_hidden)
        self.fc2 = nn.Linear(d_hidden, d_out)
        self.gate = nn.Linear(d_hidden, d_out * 2)
        self.skip = nn.Linear(d_in, d_out) if d_in != d_out else nn.Identity()
        self.norm = nn.LayerNorm(d_out)

    def forward(self, x):
        h = F.elu(self.fc1(x))
        a, b = self.gate(h).chunk(2, dim=-1)
        return self.norm(self.skip(x) + a * torch.sigmoid(b))


class TFTNet(nn.Module):
    """Compact TFT: GRN variable selection, LSTM encoder, causal self-attention,
    gated GRN, last step. (No static covariates / known-future inputs / quantile head.)"""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 16):
        super().__init__()
        self.select = _GRN(input_dim, hidden, input_dim)
        self.embed = nn.Linear(input_dim, hidden)
        self.lstm = nn.LSTM(hidden, hidden, batch_first=True)
        self.attn = nn.MultiheadAttention(hidden, 2, batch_first=True)
        self.post = _GRN(hidden, hidden, hidden)
        self.head = nn.Linear(hidden, 1)

    def forward(self, x):
        w = torch.softmax(self.select(x), dim=-1)  # per-step variable weights
        h, _ = self.lstm(self.embed(x * w))
        T = h.shape[1]
        causal = torch.triu(torch.ones(T, T, dtype=torch.bool, device=h.device), diagonal=1)
        a, _ = self.attn(h, h, h, attn_mask=causal, need_weights=False)
        return self.head(self.post(h + a)[:, -1, :])


class TSMixerNet(nn.Module):
    """Compact TSMixer: alternating time-mixing and feature-mixing MLPs."""

    def __init__(self, input_dim: int, hidden: int = 16, window: int = 16, blocks: int = 2):
        super().__init__()
        self.t_mix = nn.ModuleList(nn.Linear(window, window) for _ in range(blocks))
        self.f_mix = nn.ModuleList(
            nn.Sequential(nn.Linear(input_dim, hidden), nn.ReLU(), nn.Linear(hidden, input_dim)) for _ in range(blocks)
        )
        self.t_out = nn.Linear(window, 1)
        self.head = nn.Linear(input_dim, 1)

    def forward(self, x):
        for tm, fm in zip(self.t_mix, self.f_mix):
            x = x + F.relu(tm(x.transpose(1, 2))).transpose(1, 2)
            x = x + fm(x)
        return self.head(self.t_out(x.transpose(1, 2)).squeeze(-1))


ARCHS = {
    "lstm": LSTMNet,
    "tcn": TCNNet,
    "dlinear": DLinearNet,
    "nhits": NHiTSNet,
    "patchtst": PatchTSTNet,
    "tft": TFTNet,
    "tsmixer": TSMixerNet,
}


def build_model(arch: str, input_dim: int, hidden: int, window: int) -> nn.Module:
    if arch not in ARCHS:
        raise failures.DataError(f"unknown sequence arch {arch!r}; known: {sorted(ARCHS)}")
    return ARCHS[arch](input_dim, hidden, window)


# --------------------------------------------------------------------------- #
# Budget and the GBM comparison
# --------------------------------------------------------------------------- #
class _BudgetStop(Exception):
    pass


def budget_seconds(definition: dict) -> float:
    """`max_gpu_hours` in seconds. Required: the platform has no default (AT-66)."""
    raw = definition.get("max_gpu_hours")
    if raw is None:
        raw = (definition.get("hyperparameters") or {}).get("max_gpu_hours")
    try:
        hours = float(raw)
    except (TypeError, ValueError):
        hours = float("nan")
    if not math.isfinite(hours) or hours <= 0:
        raise failures.DataError(f"a sequence model must declare max_gpu_hours > 0, got {raw!r}")
    return hours * 3600.0


def _metric(objective: str, y, pred) -> float:
    y = np.asarray(y, dtype=float)
    pred = np.asarray(pred, dtype=float)
    if objective == "classification":
        p = np.clip(pred, 1e-6, 1 - 1e-6)
        return float(-np.mean(y * np.log(p) + (1 - y) * np.log(1 - p)))
    return float(np.mean((y - pred) ** 2))


def gbm_on_lagged_windows(objective: str, Xtr, ytr, Xte, yte, seed: int = 42) -> dict:
    """A LightGBM on the same windows flattened to lagged features, scored on the
    same rows with the same metric (MSE or log loss; lower is better)."""
    import lightgbm as lgb

    Xtr2 = Xtr.reshape(len(Xtr), -1)
    Xte2 = Xte.reshape(len(Xte), -1)
    common = dict(n_estimators=100, learning_rate=0.05, num_leaves=15, min_child_samples=5,
                  random_state=seed, n_jobs=1, verbose=-1)
    if objective == "classification":
        if len(np.unique(ytr)) < 2:
            return {"recorded": False, "reason": "single-class training labels"}
        m = lgb.LGBMClassifier(**common).fit(Xtr2, ytr)
        pred = m.predict_proba(Xte2)[:, 1]
        name = "logloss"
    else:
        m = lgb.LGBMRegressor(**common).fit(Xtr2, ytr)
        pred = m.predict(Xte2)
        name = "mse"
    return {"recorded": True, "metric": name, "score": _metric(objective, yte, pred)}


def compare_to_gbm(objective: str, model_score: float, gbm: dict) -> dict:
    """The `vs_gbm` record. `beats_gbm` needs a finite recorded GBM score and a
    strictly lower model score; anything else is False, never None-as-pass."""
    out = {"baseline": "lightgbm_lagged_windows", **gbm}
    ok = bool(gbm.get("recorded")) and math.isfinite(model_score) and math.isfinite(gbm.get("score", float("nan")))
    out["model_score"] = model_score
    out["beats_gbm"] = bool(ok and model_score < gbm["score"])
    if ok:
        out["improvement"] = float((gbm["score"] - model_score) / max(abs(gbm["score"]), 1e-12))
    return out


# --------------------------------------------------------------------------- #
# Training
# --------------------------------------------------------------------------- #
def train_variant(variant: str, definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    """Train one sequence variant on real windows. `train(definition, df, emit)`."""
    engine.seed_everything(definition)
    seq = windows.sequence_spec(definition)
    if seq is None:
        raise failures.DataError(
            f"sequence family variant {variant!r} requires declaration.sequence {{window, stride}}; "
            "refusing to train on 2-D rows"
        )
    budget = budget_seconds(definition)
    started = time.monotonic()
    deadline = started + budget

    p2d = engine.prepare(definition, df)
    p, seq = windows.apply_sequence(definition, p2d)
    hp = p.hp
    hidden = int(hp.get("hidden_dim", 16))
    epochs = int(hp.get("epochs", 30))
    lr = float(hp.get("learning_rate", 3e-3))
    batch_size = int(hp.get("batch_size", 64))
    input_dim = int(p.X_tr.shape[2])
    W = seq["window"]

    model = build_model(variant, input_dim, hidden, W)
    state = {"epochs": 0, "stopped": False}

    def budgeted(phase, pct, metric=None):
        emit_progress(phase, pct, metric)
        state["epochs"] += 1
        if time.monotonic() >= deadline and state["epochs"] < epochs:
            raise _BudgetStop()

    loss_info: dict = {}
    try:
        model, loss_info = engine.train_torch_model(
            model, p.X_tr, p.y_tr, p.X_val, p.y_val, objective=p.objective, epochs=epochs, lr=lr,
            batch_size=batch_size, emit_progress=budgeted, weight_decay=float(hp.get("weight_decay", 0.0)),
            sample_weight=p.w_tr, checkpoint=definition.get("_checkpoint"),
        )
    except _BudgetStop:
        state["stopped"] = True

    def predict(X):
        if len(X) == 0:
            return np.array([])
        model.eval()
        with torch.no_grad():
            raw = np.atleast_1d(model(torch.tensor(np.asarray(X, dtype=np.float32))).squeeze(-1).numpy().astype(float))
        return 1.0 / (1.0 + np.exp(-raw)) if p.objective == "classification" else raw

    metric_fn = engine.classification_metrics if p.objective == "classification" else engine.regression_metrics
    parts = [metric_fn("val", p.y_val, predict(p.X_val)), metric_fn("test", p.y_te, predict(p.X_te))]

    # The comparison the promotion rule needs. Scored on the test windows.
    vs_gbm: dict
    if len(p.X_te) == 0:
        vs_gbm = {"baseline": "lightgbm_lagged_windows", "recorded": False,
                  "reason": "no test windows", "beats_gbm": False}
    else:
        try:
            gbm = gbm_on_lagged_windows(p.objective, p.X_tr, p.y_tr, p.X_te, p.y_te, seed=p.seed)
        except Exception as e:  # noqa: BLE001  a failed comparison is a missing one, never a pass
            gbm = {"recorded": False, "reason": f"{type(e).__name__}: {e}"}
        vs_gbm = compare_to_gbm(p.objective, _metric(p.objective, p.y_te, predict(p.X_te)), gbm)

    metrics = engine.assemble_metrics(
        p.objective, parts, framework_version=torch.__version__,
        n_train=len(p.X_tr), n_val=len(p.X_val), n_test=len(p.X_te),
        extra={
            "family": "sequence", "variant": variant, "arch": variant, "input_dim": input_dim,
            "sequence": seq, "vs_gbm": vs_gbm, "promotable": bool(vs_gbm.get("beats_gbm")),
            "budget_seconds": budget, "wall_seconds": time.monotonic() - started,
            "budget_exhausted": state["stopped"], "epochs_run": state["epochs"], **loss_info,
        },
    )

    buf = io.BytesIO()
    torch.save({"state_dict": model.state_dict(), "arch": variant, "input_dim": input_dim,
                "hidden_dim": hidden, "objective": p.objective, "window": W}, buf)
    bundle = engine.wrap_bundle(
        buf.getvalue(), framework="torch", objective=p.objective, feature_order=p.feature_order,
        scaler=p.scaler, arch=variant, sequence=seq,
        extra={"family": "sequence", "variant": variant},
    )
    return bundle, metrics


def _entry(variant: str):
    def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
        return train_variant(variant, definition, df, emit_progress)

    train.__name__ = f"train_{variant}"
    return train


train_lstm = _entry("lstm")
train_tcn = _entry("tcn")
train_dlinear = _entry("dlinear")
train_nhits = _entry("nhits")
train_patchtst = _entry("patchtst")
train_tft = _entry("tft")
train_tsmixer = _entry("tsmixer")
