"""GARCH-t volatility adapter (I-1.6).

Fits a GARCH(1,1) model with Student-t innovations on the training return series
using the `arch` library (https://arch.readthedocs.io). Produces a distributional
volatility forecast: the predictive distribution is a scaled t-distribution
parameterised by (sigma_t, nu) where sigma_t is the conditional volatility.

The bundle stores fitted GARCH parameters so the inference service can reconstruct
the predictive distribution without re-fitting. At inference, the unconditional
volatility is used as the σ estimate when live bar data is unavailable (safe fallback;
live σ updating is deferred to a future phase).

Requires `arch` library: `pip install arch`.
"""

import pickle
import warnings

import numpy as np

from .. import engine


def _fit_garch(returns: np.ndarray, seed: int):
    """Fit GARCH(1,1)-t and return (result, sigma_uncond, nu, omega, alpha, beta)."""
    try:
        from arch import arch_model  # type: ignore
    except ImportError as exc:
        raise RuntimeError(
            "GARCH adapter requires the 'arch' package: pip install arch"
        ) from exc

    # Filter near-zero returns to avoid convergence issues.
    rets = np.asarray(returns, dtype=float)
    rets = rets[np.isfinite(rets)]
    if len(rets) < 50:
        raise ValueError(f"GARCH requires at least 50 return observations; got {len(rets)}")

    am = arch_model(rets * 100, vol="GARCH", p=1, q=1, dist="t", rescale=False)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        res = am.fit(disp="off", options={"maxiter": 500})

    params = res.params.to_dict()
    omega = float(params.get("omega", 1e-6))
    alpha_1 = float(params.get("alpha[1]", 0.05))
    beta_1 = float(params.get("beta[1]", 0.9))
    nu = float(params.get("nu", 10.0))
    nu = max(nu, 2.1)  # ensure t-distribution variance is finite

    # Unconditional variance: omega / (1 - alpha - beta)
    denom = max(1.0 - alpha_1 - beta_1, 1e-6)
    sigma_uncond = float(np.sqrt(omega / denom)) / 100.0  # rescale back to original units

    return params, sigma_uncond, nu, omega, alpha_1, beta_1


def _quantiles_from_t(sigma_t: float, nu: float, levels: list[float]) -> np.ndarray:
    """Compute quantiles of scaled Student-t(nu) × sigma_t."""
    from scipy import stats  # type: ignore
    q = stats.t.ppf(levels, df=nu)
    return q * sigma_t


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    engine.seed_everything(definition)
    p = engine.prepare(definition, df)
    levels = p.quantile_levels or [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95]

    # Extract return series from training data.
    y_tr_raw = p.y_tr.astype(float)
    emit_progress("fitting", 10.0, {"status": "fitting_garch"})

    try:
        params, sigma_uncond, nu, omega, alpha_1, beta_1 = _fit_garch(y_tr_raw, p.seed)
    except Exception as e:
        raise RuntimeError(f"GARCH fit failed: {e}") from e

    emit_progress("fitting", 70.0, {"sigma_uncond": sigma_uncond, "nu": nu})

    # Predictive quantiles using unconditional σ (serve-time fallback).
    q_sigma = _quantiles_from_t(sigma_uncond, nu, levels)
    q_return = engine.rescale_quantiles(q_sigma, sigma_uncond)
    median_return = float(np.interp(0.5, levels, q_return))

    # Validate (I-1.12)
    engine.validate_distribution(q_sigma.reshape(1, -1), levels, sigma_uncond)

    metrics = engine.assemble_metrics(
        "regression",
        [],
        framework_version=_arch_version(),
        n_train=len(p.X_tr),
        n_val=0,
        n_test=0,
        extra={
            "arch": "garch_t",
            "sigma_uncond": sigma_uncond,
            "nu": nu,
            "omega": omega,
            "alpha_1": alpha_1,
            "beta_1": beta_1,
            "n_quantiles": len(levels),
            "sigma": sigma_uncond,
        },
    )
    # CRPS from predictive distribution on test returns (approximate).
    if len(p.y_te) > 0:
        q_te = np.tile(q_sigma * sigma_uncond, (len(p.y_te), 1))
        crps = engine.pinball_loss(p.y_te.astype(float), q_te, levels)
        metrics["test_crps"] = crps
        metrics["crps"] = crps

    inner = pickle.dumps({
        "params": params,
        "sigma_uncond": sigma_uncond,
        "nu": nu,
        "omega": omega,
        "alpha_1": alpha_1,
        "beta_1": beta_1,
        "levels": levels,
    })
    bundle = engine.wrap_bundle(
        inner,
        framework="garch",
        objective="quantile",
        feature_order=p.feature_order,
        scaler=p.scaler,
        output_kind="distribution",
        quantile_levels=levels,
        sigma_scaler=sigma_uncond,
        extra={"arch_type": "garch_t"},
    )
    emit_progress("fitting", 100.0, metrics)
    return bundle, metrics


def _arch_version() -> str:
    try:
        import arch  # type: ignore
        return getattr(arch, "__version__", "unknown")
    except Exception:  # noqa: BLE001
        return "unknown"
