"""Probabilistic family: NGBoost-style Gaussian boosting, quantile regression forest, split
conformal wrapper (N-2.7). `ngboost` is not installed; the natural-gradient boosting here is our
own, using sklearn regression trees.

Acceptance (N-2.7): empirical coverage of the central 90% interval on held-out rows is within a
stated tolerance of 90% (`COVERAGE_TOLERANCE`), tested on synthetic heteroscedastic data with a
fixed seed. The conformal wrapper is also the base of N-4.1 (ACI): `SplitConformal` exposes the
calibration residuals and the interval function it needs.
"""

from __future__ import annotations

import math
import pickle

import numpy as np
import pandas as pd
from scipy import sparse, stats

from .. import engine, failures

COVERAGE_TARGET = 0.90
COVERAGE_TOLERANCE = 0.05
DEFAULT_LEVELS = [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95]


# --------------------------------------------------------------------------- #
# Gaussian natural-gradient boosting
# --------------------------------------------------------------------------- #
class GaussianNGB:
    """y ~ N(mu(x), sigma(x)^2); mu and log sigma are each a sum of shrunken trees fitted to
    the NATURAL gradient of the Gaussian NLL (Duan et al. 2020):

        d/dmu: mu - y                       (gradient (mu-y)/s^2, Fisher 1/s^2)
        d/dlog s: (1 - (y-mu)^2/s^2) / 2    (gradient 1-(y-mu)^2/s^2, Fisher 2)
    """

    def __init__(self, n_estimators=200, learning_rate=0.05, max_depth=3, min_samples_leaf=10,
                 subsample=0.8, seed=42):
        self.n_estimators, self.lr, self.max_depth = n_estimators, learning_rate, max_depth
        self.min_leaf, self.subsample, self.seed = min_samples_leaf, subsample, seed
        self.trees_mu, self.trees_ls = [], []

    def fit(self, X, y, X_val=None, y_val=None, patience=20):
        from sklearn.tree import DecisionTreeRegressor

        X = np.asarray(X, float)
        y = np.asarray(y, float)
        self.m0 = float(y.mean())
        self.s0 = float(y.std()) or 1.0
        ys = (y - self.m0) / self.s0
        rng = np.random.default_rng(self.seed)
        mu = np.zeros(len(y))
        ls = np.full(len(y), 0.0)  # log sigma in standardized units: sigma starts at 1 == std(y)
        best, best_i, since = np.inf, 0, 0
        for i in range(self.n_estimators):
            s2 = np.exp(2 * ls)
            g_mu = mu - ys
            g_ls = 0.5 * (1.0 - (ys - mu) ** 2 / s2)
            idx = rng.random(len(y)) < self.subsample
            if idx.sum() < 2 * self.min_leaf:
                idx[:] = True
            t_mu = DecisionTreeRegressor(max_depth=self.max_depth, min_samples_leaf=self.min_leaf,
                                         random_state=self.seed + i).fit(X[idx], -g_mu[idx])
            t_ls = DecisionTreeRegressor(max_depth=self.max_depth, min_samples_leaf=self.min_leaf,
                                         random_state=self.seed + 7919 + i).fit(X[idx], -g_ls[idx])
            self.trees_mu.append(t_mu)
            self.trees_ls.append(t_ls)
            mu = mu + self.lr * t_mu.predict(X)
            ls = ls + self.lr * t_ls.predict(X)
            if X_val is not None and len(X_val) > 0:
                m, s = self.predict_dist(X_val)
                nll = float(np.mean(0.5 * np.log(2 * np.pi * s**2) + (y_val - m) ** 2 / (2 * s**2)))
                if nll < best - 1e-9:
                    best, best_i, since = nll, i + 1, 0
                else:
                    since += 1
                    if since >= patience:
                        break
        if X_val is not None and len(X_val) > 0 and best_i > 0:
            self.trees_mu, self.trees_ls = self.trees_mu[:best_i], self.trees_ls[:best_i]
        return self

    def predict_dist(self, X):
        X = np.asarray(X, float)
        mu = np.zeros(len(X))
        ls = np.zeros(len(X))
        for tm, tl in zip(self.trees_mu, self.trees_ls):
            mu += self.lr * tm.predict(X)
            ls += self.lr * tl.predict(X)
        ls = np.clip(ls, -8.0, 8.0)
        return self.m0 + self.s0 * mu, self.s0 * np.exp(ls)

    def predict_quantiles(self, X, levels):
        m, s = self.predict_dist(X)
        return m[:, None] + s[:, None] * stats.norm.ppf(levels)[None, :]


# --------------------------------------------------------------------------- #
# Quantile regression forest (Meinshausen 2006)
# --------------------------------------------------------------------------- #
class QRF:
    """Random forest whose prediction at x is the weighted empirical distribution of ALL training
    targets, weight = share of trees in which a sample lands in x's leaf (1/leaf size)."""

    def __init__(self, n_estimators=100, min_samples_leaf=5, max_features=0.5, seed=42):
        self.n_estimators, self.min_leaf, self.max_features, self.seed = (
            n_estimators, min_samples_leaf, max_features, seed)

    def fit(self, X, y):
        from sklearn.ensemble import RandomForestRegressor

        X = np.asarray(X, float)
        self.y = np.asarray(y, float)
        self.rf = RandomForestRegressor(
            n_estimators=self.n_estimators, min_samples_leaf=self.min_leaf,
            max_features=self.max_features, random_state=self.seed, n_jobs=1,
        ).fit(X, self.y)
        leaves = self.rf.apply(X)  # (n, T)
        self._train_w = []
        for t in range(leaves.shape[1]):
            lt = leaves[:, t]
            _u, inv, cnt = np.unique(lt, return_inverse=True, return_counts=True)
            self._train_w.append((_u, sparse.csr_matrix(
                (1.0 / cnt[inv], (np.arange(len(lt)), inv)), shape=(len(lt), len(_u)))))
        return self

    def weights(self, X):
        leaves = self.rf.apply(np.asarray(X, float))
        W = None
        for t, (u, M) in enumerate(self._train_w):
            pos = np.searchsorted(u, leaves[:, t])
            pos = np.clip(pos, 0, len(u) - 1)
            hit = u[pos] == leaves[:, t]
            Q = sparse.csr_matrix((hit.astype(float), (np.arange(len(pos)), pos)), shape=(len(pos), len(u)))
            part = (Q @ M.T).toarray()
            W = part if W is None else W + part
        return W / self.n_estimators

    def predict_quantiles(self, X, levels):
        W = self.weights(X)
        order = np.argsort(self.y)
        ys = self.y[order]
        Wc = np.cumsum(W[:, order], axis=1)
        Wc /= Wc[:, -1:]
        out = np.empty((len(W), len(levels)))
        for j, q in enumerate(levels):
            idx = (Wc >= q).argmax(axis=1)
            out[:, j] = ys[idx]
        return out

    def predict_point(self, X):
        W = self.weights(X)
        return W @ self.y


# --------------------------------------------------------------------------- #
# Split conformal
# --------------------------------------------------------------------------- #
class SplitConformal:
    """Split-conformal wrapper around any point predictor `base.predict(X)`.

    `fit_calibration` stores signed residuals y - base(x) from a calibration set that the base
    model never trained on. `interval(X, alpha)` returns the central (1-alpha) interval with the
    finite-sample corrected quantile ceil((n+1)(1-alpha))/n of |residual|.
    """

    def __init__(self, base):
        self.base = base
        self.resid = np.empty(0)

    def fit_calibration(self, X_cal, y_cal):
        self.resid = np.asarray(y_cal, float) - np.asarray(self.base.predict(np.asarray(X_cal, float)), float)
        if len(self.resid) < 20:
            raise failures.DataError("conformal calibration needs at least 20 held-out rows")
        return self

    def half_width(self, alpha: float) -> float:
        n = len(self.resid)
        k = min(math.ceil((n + 1) * (1 - alpha)), n)
        return float(np.sort(np.abs(self.resid))[k - 1])

    def interval(self, X, alpha=0.10):
        p = np.asarray(self.base.predict(np.asarray(X, float)), float)
        h = self.half_width(alpha)
        return p - h, p + h

    def predict_quantiles(self, X, levels):
        p = np.asarray(self.base.predict(np.asarray(X, float)), float)
        n = len(self.resid)
        qs = np.quantile(self.resid, [min(max(l, 0.0), 1.0) for l in levels], method="higher") if n else np.zeros(len(levels))
        return p[:, None] + qs[None, :]


class _GBRBase:
    def __init__(self, seed=42):
        from sklearn.ensemble import GradientBoostingRegressor

        self.m = GradientBoostingRegressor(n_estimators=150, max_depth=3, learning_rate=0.05,
                                           subsample=0.8, random_state=seed)

    def fit(self, X, y):
        self.m.fit(X, y)
        return self

    def predict(self, X):
        return self.m.predict(X)


def coverage(y, lo, hi) -> float:
    y = np.asarray(y, float)
    return float(np.mean((y >= lo) & (y <= hi)))


# --------------------------------------------------------------------------- #
# Registry entry points
# --------------------------------------------------------------------------- #
def _prepare(definition, df):
    d = dict(definition)
    hp = dict(d.get("hyperparameters") or {})
    hp["objective"] = "regression"
    d["hyperparameters"] = hp
    engine.seed_everything(d)
    p = engine.prepare(d, df)
    levels = p.quantile_levels or DEFAULT_LEVELS
    return p, sorted(levels), hp


def _finish(kind, model, p, levels, hp, emit, point_extra=None):
    q_te = None
    extra = {"probabilistic": kind, "n_quantiles": len(levels)}
    if len(p.X_te):
        q_te = model.predict_quantiles(p.X_te, levels)
        q_te = np.sort(q_te, axis=1)
        crps = engine.pinball_loss(p.y_te.astype(float), q_te, levels)
        extra.update(test_crps=crps, crps=crps)
        lo = q_te[:, int(np.argmin(np.abs(np.array(levels) - 0.05)))]
        hi = q_te[:, int(np.argmin(np.abs(np.array(levels) - 0.95)))]
        if 0.05 in levels and 0.95 in levels:
            extra["coverage_90"] = coverage(p.y_te, lo, hi)
    metrics = engine.assemble_metrics("regression", [], framework_version="prob-1",
                                      n_train=len(p.X_tr), n_val=len(p.X_val), n_test=len(p.X_te), extra=extra)
    bundle = engine.wrap_bundle(
        pickle.dumps({"kind": kind, "model": model, "levels": levels}),
        framework="probabilistic", objective="quantile", feature_order=p.feature_order,
        scaler=p.scaler, output_kind="distribution", quantile_levels=levels, sigma_scaler=p.sigma,
        extra={"family": "probabilistic", "variant": kind},
    )
    emit("fitting", 100.0, metrics)
    return bundle, metrics


def train_ngboost(definition, df, emit):
    p, levels, hp = _prepare(definition, df)
    emit("fitting", 10.0, {"probabilistic": "ngboost"})
    m = GaussianNGB(n_estimators=int(hp.get("n_estimators", 200)), learning_rate=float(hp.get("learning_rate", 0.05)),
                    max_depth=int(hp.get("max_depth", 3)), seed=p.seed)
    m.fit(p.X_tr, p.y_tr.astype(float), p.X_val, p.y_val.astype(float))
    return _finish("ngboost", m, p, levels, hp, emit)


def train_qrf(definition, df, emit):
    p, levels, hp = _prepare(definition, df)
    emit("fitting", 10.0, {"probabilistic": "qrf"})
    m = QRF(n_estimators=int(hp.get("n_estimators", 100)), min_samples_leaf=int(hp.get("min_samples_leaf", 5)),
            seed=p.seed).fit(p.X_tr, p.y_tr.astype(float))
    return _finish("qrf", m, p, levels, hp, emit)


def train_conformal(definition, df, emit):
    p, levels, hp = _prepare(definition, df)
    emit("fitting", 10.0, {"probabilistic": "conformal"})
    base = _GBRBase(seed=p.seed).fit(p.X_tr, p.y_tr.astype(float))
    sc = SplitConformal(base).fit_calibration(p.X_val, p.y_val.astype(float))
    return _finish("conformal", sc, p, levels, hp, emit)
