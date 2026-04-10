"""
Multi-branch MLP policy actor (human policy spec §10.2–10.4).

Encodes forecast / portfolio / execution / risk groups separately, concatenates, outputs target_exposure ∈ [-1, 1].
"""

from __future__ import annotations

import numpy as np

from policy_model.objects import PolicyAction, PolicyObservation


class MultiBranchMLPPolicy:
    def __init__(self, seed: int = 0) -> None:
        self._rng = np.random.default_rng(seed)
        d_f, d_p, d_e, d_r = 64, 32, 16, 16
        self._w_f = self._rng.normal(0, 0.1, size=(d_f, 32))
        self._w_p = self._rng.normal(0, 0.1, size=(d_p, 32))
        self._w_e = self._rng.normal(0, 0.1, size=(d_e, 32))
        self._w_r = self._rng.normal(0, 0.1, size=(d_r, 32))
        self._w_out = self._rng.normal(0, 0.1, size=(128, 1))

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
