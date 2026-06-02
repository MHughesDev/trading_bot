"""
Multi-branch MLP policy actor (human policy spec §10.2–10.4).

Encodes forecast / portfolio / execution / risk groups separately, concatenates, outputs target_exposure ∈ [-1, 1].
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from policy_model.objects import PolicyAction, PolicyObservation


class MultiBranchMLPPolicy:
    def __init__(self, seed: int = 0) -> None:
        self._rng = np.random.default_rng(seed)
        d_f, d_p, d_e, d_r = 64, 32, 16, 16
        self._d_f, self._d_p, self._d_e, self._d_r = d_f, d_p, d_e, d_r
        self._hidden = 32
        self._w_f = self._rng.normal(0, 0.1, size=(d_f, self._hidden))
        self._w_p = self._rng.normal(0, 0.1, size=(d_p, self._hidden))
        self._w_e = self._rng.normal(0, 0.1, size=(d_e, self._hidden))
        self._w_r = self._rng.normal(0, 0.1, size=(d_r, self._hidden))
        self._w_out = self._rng.normal(0, 0.1, size=(4 * self._hidden, 1))

    def _enc(self, x: np.ndarray, W: np.ndarray) -> np.ndarray:
        if len(x) < W.shape[0]:
            x = np.pad(x, (0, W.shape[0] - len(x)))
        x = x[: W.shape[0]]
        return np.tanh(x @ W)

    def forward(self, obs: PolicyObservation) -> dict:
        ff = np.asarray(obs.forecast_features, dtype=np.float64)
        pf = np.asarray(obs.portfolio_features, dtype=np.float64)
        ef = np.asarray(obs.execution_features, dtype=np.float64)
        rf = np.asarray(obs.risk_features, dtype=np.float64)
        zf = self._enc(ff, self._w_f)
        zp = self._enc(pf, self._w_p)
        ze = self._enc(ef, self._w_e)
        zr = self._enc(rf, self._w_r)
        z = np.concatenate([zf, zp, ze, zr])
        raw = float(np.tanh((z @ self._w_out).squeeze()))
        return {"action_params": raw, "policy_latent": z, "diagnostics": {}}

    def select_action(self, obs: PolicyObservation, *, deterministic: bool = True) -> PolicyAction:
        out = self.forward(obs)
        a = float(out["action_params"])
        if not deterministic:
            a = float(np.clip(a + self._rng.normal(0, 0.05), -1, 1))
        return PolicyAction(target_exposure=a, action_diagnostics={"deterministic": deterministic})

    # --- training: analytic backprop for the 4-branch tanh MLP ---

    def _branch_inputs(self, obs: PolicyObservation) -> list[tuple[np.ndarray, np.ndarray, str]]:
        return [
            (np.asarray(obs.forecast_features, dtype=np.float64), self._w_f, "w_f"),
            (np.asarray(obs.portfolio_features, dtype=np.float64), self._w_p, "w_p"),
            (np.asarray(obs.execution_features, dtype=np.float64), self._w_e, "w_e"),
            (np.asarray(obs.risk_features, dtype=np.float64), self._w_r, "w_r"),
        ]

    def forward_with_cache(self, obs: PolicyObservation) -> tuple[float, dict]:
        """Forward pass that retains intermediates needed by :meth:`backward`."""
        xs: list[np.ndarray] = []
        zs: list[np.ndarray] = []
        for x, W, _name in self._branch_inputs(obs):
            if len(x) < W.shape[0]:
                x = np.pad(x, (0, W.shape[0] - len(x)))
            x = x[: W.shape[0]]
            xs.append(x)
            zs.append(np.tanh(x @ W))
        z = np.concatenate(zs)
        s = float((z @ self._w_out).squeeze())
        a = float(np.tanh(s))
        return a, {"xs": xs, "zs": zs, "z": z, "a": a}

    def backward(self, cache: dict, d_loss_d_a: float) -> dict[str, np.ndarray]:
        """Gradients of a scalar loss w.r.t. every weight, given dL/da."""
        a = cache["a"]
        z = cache["z"]
        d_s = d_loss_d_a * (1.0 - a * a)  # through output tanh
        grad_w_out = z.reshape(-1, 1) * d_s  # (4*hidden, 1)
        d_z = self._w_out.reshape(-1) * d_s  # (4*hidden,)
        grads: dict[str, np.ndarray] = {"w_out": grad_w_out}
        names = ("w_f", "w_p", "w_e", "w_r")
        for i, name in enumerate(names):
            zb = cache["zs"][i]
            xb = cache["xs"][i]
            d_zb = d_z[i * self._hidden : (i + 1) * self._hidden]
            d_pre = d_zb * (1.0 - zb * zb)  # through branch tanh
            grads[name] = np.outer(xb, d_pre)
        return grads

    def apply_grads(self, grads: dict[str, np.ndarray], lr: float) -> None:
        """In-place SGD step."""
        self._w_f -= lr * grads["w_f"]
        self._w_p -= lr * grads["w_p"]
        self._w_e -= lr * grads["w_e"]
        self._w_r -= lr * grads["w_r"]
        self._w_out -= lr * grads["w_out"]

    def save(self, path: str | Path) -> None:
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(
            p,
            w_f=self._w_f,
            w_p=self._w_p,
            w_e=self._w_e,
            w_r=self._w_r,
            w_out=self._w_out,
            d_f=self._d_f,
            d_p=self._d_p,
            d_e=self._d_e,
            d_r=self._d_r,
            hidden=self._hidden,
        )

    def load(self, path: str | Path) -> None:
        p = Path(path)
        with np.load(p, allow_pickle=False) as z:
            self._w_f = np.asarray(z["w_f"])
            self._w_p = np.asarray(z["w_p"])
            self._w_e = np.asarray(z["w_e"])
            self._w_r = np.asarray(z["w_r"])
            self._w_out = np.asarray(z["w_out"])
            self._d_f = int(z["d_f"])
            self._d_p = int(z["d_p"])
            self._d_e = int(z["d_e"])
            self._d_r = int(z["d_r"])
            self._hidden = int(z["hidden"])
