"""Regime family: Gaussian HMM and statistical jump models, FILTERED only (N-2.4).

Artifact type `regime_labels`. The label at time t is a function of observations <= t and
nothing else:

  * HMM: the forward algorithm's filtered state probabilities. EM (Baum-Welch) uses a backward
    pass internally to *estimate parameters on the training window*; no backward/smoothed/Viterbi
    quantity is ever exposed as a label, and `filter_*` is the only way to produce one.
  * Jump models (JM, continuous JM, sparse JM): centroids and the jump penalty lambda are
    estimated offline on the training window by alternating DP / centroid updates; the label at t
    is the argmin of the *forward* DP value V_t(k) = loss_t(k) + min(V_{t-1}(k), min_j V_{t-1}(j) + lambda),
    which sees no future bar (the "online" jump model).

There is no smoothed-label function: `SMOOTHED_PATH_EXISTS` is False and a test asserts the module
exposes no smoothing/Viterbi symbol. A truncation test asserts labels[:t] are identical when later
bars are appended.
"""

from __future__ import annotations

import pickle

import numpy as np
import pandas as pd

from .. import engine, failures

SMOOTHED_PATH_EXISTS = False
KINDS = ("hmm", "jm", "cjm", "sjm")
ARTIFACT_TYPE = "regime_labels"


# --------------------------------------------------------------------------- #
# Observations
# --------------------------------------------------------------------------- #
def observation_matrix(definition: dict, df: pd.DataFrame) -> tuple[np.ndarray, list[str]]:
    """Feature matrix the regime is read from. Never includes the label (it is a forward
    quantity)."""
    cfg = definition.get("regime") or {}
    cols = cfg.get("features")
    if cols:
        missing = [c for c in cols if c not in df.columns]
        if missing:
            raise failures.DataError(f"regime features missing from dataset: {missing}")
        return df[cols].to_numpy(float), list(cols)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    feats = engine.feature_frame(df, label_col)
    if feats.shape[1] > 0:
        return feats.to_numpy(float), list(feats.columns)
    if "close" in df.columns:
        c = df["close"].to_numpy(float)
        r = np.concatenate([[0.0], np.diff(np.log(np.maximum(c, 1e-12)))])
        return np.column_stack([r, np.abs(r)]), ["ret", "abs_ret"]
    raise failures.DataError("no regime observations: provide features or a close column")


def _standardize(X: np.ndarray, train: slice) -> tuple[np.ndarray, np.ndarray]:
    Xt = X[train]
    Xt = Xt[np.all(np.isfinite(Xt), axis=1)]
    if len(Xt) < 30:
        raise failures.DataError("regime fit needs at least 30 finite training rows")
    return engine.fit_scaler(Xt)


def _prep(X, state):
    Z = (np.asarray(X, float) - state["mean"]) / state["std"]
    return np.nan_to_num(Z, nan=0.0, posinf=0.0, neginf=0.0)


# --------------------------------------------------------------------------- #
# Gaussian HMM
# --------------------------------------------------------------------------- #
def _log_emission(Z, mu, var):
    d = Z[:, None, :] - mu[None, :, :]
    return -0.5 * np.sum(np.log(2 * np.pi * var)[None] + d**2 / var[None], axis=2)


def _forward(logB, pi, A):
    """Scaled forward pass. Returns (alpha_hat (T,K), c (T,), loglik)."""
    T, K = logB.shape
    m = logB.max(axis=1, keepdims=True)
    B = np.exp(logB - m)
    alpha = np.empty((T, K))
    c = np.empty(T)
    a = pi * B[0]
    c[0] = a.sum()
    alpha[0] = a / c[0]
    for t in range(1, T):
        a = (alpha[t - 1] @ A) * B[t]
        c[t] = a.sum()
        alpha[t] = a / max(c[t], 1e-300)
    return alpha, c, float(np.sum(np.log(np.maximum(c, 1e-300))) + m.sum()), B


def _em_fit(Z, K, seed, iters=60):
    """Baum-Welch. The backward pass here is parameter estimation on the training window only."""
    rng = np.random.default_rng(seed)
    T, D = Z.shape
    order = np.argsort(Z[:, 0], kind="stable")
    mu = np.empty((K, D))
    var = np.empty((K, D))
    for k, idx in enumerate(np.array_split(order, K)):
        mu[k] = Z[idx].mean(0) + 1e-3 * rng.standard_normal(D)
        var[k] = np.maximum(Z[idx].var(0), 1e-3)
    A = np.full((K, K), 0.05 / max(K - 1, 1))
    np.fill_diagonal(A, 0.95)
    pi = np.full(K, 1.0 / K)
    prev = -np.inf
    for _ in range(iters):
        logB = _log_emission(Z, mu, var)
        alpha, c, ll, B = _forward(logB, pi, A)
        beta = np.ones((T, K))
        for t in range(T - 2, -1, -1):
            beta[t] = (A @ (B[t + 1] * beta[t + 1])) / max(c[t + 1], 1e-300)
        gamma = alpha * beta
        gamma /= gamma.sum(1, keepdims=True)
        xi = np.zeros((K, K))
        for t in range(T - 1):
            x = alpha[t][:, None] * A * (B[t + 1] * beta[t + 1])[None, :]
            xi += x / max(x.sum(), 1e-300)
        pi = gamma[0] + 1e-9
        pi /= pi.sum()
        A = xi + 1e-9
        A /= A.sum(1, keepdims=True)
        w = gamma.sum(0) + 1e-9
        mu = (gamma.T @ Z) / w[:, None]
        var = np.maximum((gamma.T @ (Z**2)) / w[:, None] - mu**2, 1e-4)
        if abs(ll - prev) < 1e-6 * max(abs(ll), 1.0):
            break
        prev = ll
    # order states by total variance, so label 0 is the calmest, label K-1 the most volatile
    o = np.argsort(var.sum(1) + (mu**2).sum(1) * 0.0, kind="stable")
    return {"pi": pi[o], "A": A[np.ix_(o, o)], "mu": mu[o], "var": var[o], "loglik": ll}


def filter_hmm(state: dict, X) -> tuple[np.ndarray, np.ndarray]:
    """Filtered probabilities P(s_t | x_<=t) and argmax labels. Forward pass only."""
    Z = _prep(X, state)
    logB = _log_emission(Z, state["mu"], state["var"])
    alpha, _c, _ll, _B = _forward(logB, state["pi"], state["A"])
    return alpha.argmax(1), alpha


# --------------------------------------------------------------------------- #
# Jump models
# --------------------------------------------------------------------------- #
def _losses(Z, mu, w=None):
    d2 = (Z[:, None, :] - mu[None, :, :]) ** 2
    if w is not None:
        d2 = d2 * w[None, None, :]
    return d2.sum(2)


def _dp_offline(loss, lam):
    """Optimal state sequence minimising loss + lam * jumps. Used ONLY to fit centroids on the
    training window."""
    T, K = loss.shape
    V = np.empty((T, K))
    back = np.zeros((T, K), dtype=int)
    V[0] = loss[0]
    for t in range(1, T):
        best_j = int(np.argmin(V[t - 1]))
        stay = V[t - 1]
        jump = V[t - 1][best_j] + lam
        use_stay = stay <= jump
        V[t] = loss[t] + np.where(use_stay, stay, jump)
        back[t] = np.where(use_stay, np.arange(K), best_j)
    s = np.empty(T, dtype=int)
    s[-1] = int(np.argmin(V[-1]))
    for t in range(T - 1, 0, -1):
        s[t - 1] = back[t, s[t]]
    return s


def _forward_values(loss, lam):
    """Causal forward values V_t(k); label_t = argmin_k V_t(k) depends on bars <= t only."""
    T, K = loss.shape
    V = np.empty((T, K))
    V[0] = loss[0]
    for t in range(1, T):
        V[t] = loss[t] + np.minimum(V[t - 1], V[t - 1].min() + lam)
        V[t] -= V[t].min()  # shift; argmin is unchanged and values stay bounded
    return V


def _sparse_weights(Z, s, K, budget):
    """Witten-Tibshirani L1-constrained feature weights from per-feature between-state SS."""
    D = Z.shape[1]
    tot = ((Z - Z.mean(0)) ** 2).sum(0)
    within = np.zeros(D)
    for k in range(K):
        m = s == k
        if m.any():
            within += ((Z[m] - Z[m].mean(0)) ** 2).sum(0)
    a = np.maximum(tot - within, 0.0)
    if a.sum() <= 0:
        return np.full(D, 1.0 / np.sqrt(D))
    budget = min(max(budget, 1.0), np.sqrt(D))

    def w_of(delta):
        w = np.maximum(a - delta, 0.0)
        n = np.linalg.norm(w)
        return w / n if n > 0 else w

    w = w_of(0.0)
    if w.sum() <= budget:
        return w
    lo, hi = 0.0, a.max()
    for _ in range(60):
        mid = (lo + hi) / 2
        if w_of(mid).sum() > budget:
            lo = mid
        else:
            hi = mid
    return w_of(hi)


def _fit_jump(Z, K, lam, seed, kind, sparse_budget=None, iters=30):
    rng = np.random.default_rng(seed)
    T, D = Z.shape
    order = np.argsort(Z[:, 0], kind="stable")
    mu = np.vstack([Z[idx].mean(0) for idx in np.array_split(order, K)])
    mu = mu + 1e-3 * rng.standard_normal(mu.shape)
    w = np.full(D, 1.0 / np.sqrt(D)) if kind == "sjm" else None
    s = np.zeros(T, dtype=int)
    for _ in range(iters):
        loss = _losses(Z, mu, w)
        s_new = _dp_offline(loss, lam)
        for k in range(K):
            m = s_new == k
            if m.any():
                mu[k] = Z[m].mean(0)
        if kind == "sjm":
            w = _sparse_weights(Z, s_new, K, sparse_budget or np.sqrt(D) * 0.6)
        if np.array_equal(s_new, s):
            break
        s = s_new
    # order states by total centroid dispersion (calm -> volatile)
    o = np.argsort(((mu - Z.mean(0)) ** 2).sum(1) if w is None else (w * (mu - Z.mean(0)) ** 2).sum(1), kind="stable")
    mu = mu[o]
    loss = _losses(Z, mu, w)
    scale = float(np.median(loss.min(1))) or 1.0
    return {"mu": mu, "w": w, "lam": float(lam), "scale": scale}


def filter_jump(state: dict, X) -> tuple[np.ndarray, np.ndarray]:
    """Online jump-model labels and state weights; causal."""
    Z = _prep(X, state)
    loss = _losses(Z, state["mu"], state["w"])
    V = _forward_values(loss, state["lam"])
    labels = V.argmin(1)
    if state["kind"] == "cjm":
        # Continuous JM: soft state membership from the forward values.
        tau = max(state["scale"], 1e-6)
        P = np.exp(-(V - V.min(1, keepdims=True)) / tau)
    else:
        P = (V == V.min(1, keepdims=True)).astype(float)
    P /= P.sum(1, keepdims=True)
    return labels, P


def filter_labels(state: dict, X) -> tuple[np.ndarray, np.ndarray]:
    return filter_hmm(state, X) if state["kind"] == "hmm" else filter_jump(state, X)


# --------------------------------------------------------------------------- #
# Fit + registry entry points
# --------------------------------------------------------------------------- #
def fit(kind: str, X, *, n_states: int = 2, lam: float | None = None, seed: int = 42,
        sparse_budget: float | None = None) -> dict:
    if kind not in KINDS:
        raise failures.DataError(f"unknown regime kind {kind!r}")
    X = np.asarray(X, float)
    mean, std = _standardize(X, slice(0, len(X)))
    state = {"kind": kind, "n_states": int(n_states), "mean": mean, "std": std, "artifact_type": ARTIFACT_TYPE}
    Z = _prep(X, state)
    if kind == "hmm":
        state.update(_em_fit(Z, n_states, seed))
    else:
        lam = float(lam if lam is not None else 5.0 * Z.shape[1])
        state.update(_fit_jump(Z, n_states, lam, seed, kind, sparse_budget))
    return state


def _train(kind: str, definition: dict, df: pd.DataFrame, emit):
    engine.seed_everything(definition)
    cfg = definition.get("regime") or {}
    X, cols = observation_matrix(definition, df)
    wf = definition.get("_wf_fold")
    if wf:
        tr, te = slice(wf["train_start"], wf["train_end"]), slice(wf["test_start"], wf["test_end"])
    else:
        tr, _va, te = engine.split_indices(len(df), embargo=int(definition.get("_embargo_bars", 0) or 0))
    emit("fitting", 10.0, {"regime": kind})
    state = fit(kind, X[tr], n_states=int(cfg.get("n_states", 2)), lam=cfg.get("lam"),
                seed=engine.resolve_seed(definition), sparse_budget=cfg.get("sparse_budget"))
    state["feature_order"] = cols
    labels, probs = filter_labels(state, X)
    lab_te = labels[te]
    switches = int(np.sum(lab_te[1:] != lab_te[:-1])) if len(lab_te) > 1 else 0
    extra = {
        "regime_kind": kind, "artifact_type": ARTIFACT_TYPE, "n_states": state["n_states"],
        "filtered_only": True,
        "occupancy_test": [float(np.mean(lab_te == k)) for k in range(state["n_states"])] if len(lab_te) else [],
        "switches_test": switches,
        "mean_dwell_test": float(len(lab_te) / (switches + 1)) if len(lab_te) else 0.0,
    }
    if kind == "hmm":
        extra["loglik"] = state["loglik"]
    if kind == "sjm":
        extra["feature_weights"] = {c: float(w) for c, w in zip(cols, state["w"])}
    metrics = engine.assemble_metrics("classification", [], framework_version="regime-1",
                                      n_train=len(X[tr]), n_val=0, n_test=len(lab_te), extra=extra)
    bundle = engine.wrap_bundle(
        pickle.dumps(state), framework="regime", objective="classification", feature_order=cols,
        scaler={"mean": state["mean"].tolist(), "std": state["std"].tolist()},
        output_kind=ARTIFACT_TYPE,
        extra={"artifact_type": ARTIFACT_TYPE, "family": "regime", "variant": kind, "filtered_only": True},
    )
    emit("fitting", 100.0, metrics)
    return bundle, metrics


def train_hmm(definition, df, emit):
    return _train("hmm", definition, df, emit)


def train_jm(definition, df, emit):
    return _train("jm", definition, df, emit)


def train_cjm(definition, df, emit):
    return _train("cjm", definition, df, emit)


def train_sjm(definition, df, emit):
    return _train("sjm", definition, df, emit)
