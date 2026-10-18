"""Bring-your-own models (Set N, N-2.9; FEAT-006 section 5, test M2).

A user model is a code snapshot (a directory) plus a dataset. It implements
`ModelInterface` (`fit`, `predict`, `save`, `load`; see `byo_interface.py`), is
named by `definition["byo"]["entry"] = "module:Class"`, and runs here only inside
a sandbox subprocess (`byo_runner.py`):

  * no network (any socket event raises and is recorded; a model that swallows the
    exception still fails the run because the parent reads the violation log);
  * the snapshot directory is read-only; writes go to a scratch directory only;
  * a wall-clock timeout (`byo.timeout_s`, default 300).

For each fold a model is fitted on that fold's train rows and scored on its
validation and test rows; then one final refit is fitted on every row the folds
used to fit or calibrate (train start to the last calibration end). The bundle
carries the final model, every fold model, the snapshot itself (so serving
re-runs the identical code) and `code_hash`, the sha256 of the snapshot. Serving
refuses a bundle whose snapshot no longer hashes to `code_hash`.

`feature_order` in the bundle is the list of DATA-006 `feature_hash` values when
`byo.feature_hashes` maps column names to hashes (every column must be covered);
otherwise the column names, with `feature_order_source = "column_name"` recorded
so nobody reads a column name as a hash. The model is only ever handed a frame of
those columns, so reading any other feature fails at fit/predict with
`UndeclaredFeatureRead`.

Entry point: `train(definition, df, emit) -> (bundle_bytes, metrics)`.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import pathlib
import pickle
import shutil
import stat
import subprocess
import sys
import tempfile

import numpy as np
import pandas as pd

from . import engine, failures
from .byo_interface import ModelInterface, UndeclaredFeature  # noqa: F401  (re-exported)

TRAINER_ROOT = str(pathlib.Path(__file__).resolve().parents[1])
RUNNER = str(pathlib.Path(__file__).resolve().with_name("byo_runner.py"))
DEFAULT_TIMEOUT_S = 300


class SandboxViolation(failures.TrainerFailure):
    """The model tried to do something the sandbox forbids (network, process, writes)."""

    terminal = failures.DEPENDENCY_FAILURE


class UndeclaredFeatureRead(failures.DataError):
    """The model read a feature that is not in `feature_order`."""


class ByoModelError(failures.TrainerFailure):
    """The submitted model raised."""

    terminal = failures.DEPENDENCY_FAILURE


# --------------------------------------------------------------------------- #
# Snapshot
# --------------------------------------------------------------------------- #
def _snapshot_files(root: str) -> dict[str, bytes]:
    out: dict[str, bytes] = {}
    base = pathlib.Path(root)
    for p in sorted(base.rglob("*")):
        if p.is_file() and "__pycache__" not in p.parts and p.suffix != ".pyc":
            out[p.relative_to(base).as_posix()] = p.read_bytes()
    return out


def code_hash(files: dict[str, bytes]) -> str:
    """sha256 over the sorted (path, sha256(content)) pairs of the snapshot."""
    h = hashlib.sha256()
    for rel in sorted(files):
        h.update(rel.encode("utf-8") + b"\0" + hashlib.sha256(files[rel]).digest())
    return h.hexdigest()


def _materialize(files: dict[str, bytes], dest: str) -> None:
    for rel, data in files.items():
        p = pathlib.Path(dest) / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        os.chmod(p, stat.S_IREAD)  # read-only snapshot; the child's write guard enforces the rest


def _cleanup(path: str) -> None:
    def _rw(func, p, _exc):
        try:
            os.chmod(p, stat.S_IWRITE)
            func(p)
        except OSError:
            pass

    shutil.rmtree(path, onerror=_rw)


# --------------------------------------------------------------------------- #
# Running the child
# --------------------------------------------------------------------------- #
def _child_env() -> dict:
    keep = ("SYSTEMROOT", "SystemRoot", "PATH", "TEMP", "TMP", "LANG", "LC_ALL", "COMSPEC", "PATHEXT",
            "APPDATA", "LOCALAPPDATA", "USERPROFILE", "HOMEDRIVE", "HOMEPATH", "HOME")
    env = {k: os.environ[k] for k in keep if k in os.environ}
    env.update({"PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "0", "NO_PROXY": "*",
                "OMP_NUM_THREADS": "1"})
    return env  # no proxies, no credentials, nothing from the parent's environment


def run_sandboxed(
    op: str,
    entry: str,
    files: dict[str, bytes],
    feature_order: list[str],
    arrays: dict[str, np.ndarray],
    predict_sets: list[str],
    params: dict | None = None,
    model_in: dict[str, bytes] | None = None,
    timeout_s: float = DEFAULT_TIMEOUT_S,
) -> tuple[dict[str, np.ndarray], dict[str, bytes]]:
    """Run one job in the sandbox child. Returns ``(predictions, model_files)``."""
    work = tempfile.mkdtemp(prefix="byo_")
    snap = tempfile.mkdtemp(prefix="byo_snap_")
    try:
        _materialize(files, snap)
        np.savez(os.path.join(work, "data.npz"), **arrays)
        if model_in is not None:
            for rel, data in model_in.items():
                p = pathlib.Path(work) / "model_in" / rel
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(data)
        with open(os.path.join(work, "job.json"), "w", encoding="utf-8") as f:
            json.dump({"op": op, "entry": entry, "snapshot_dir": snap, "trainer_root": TRAINER_ROOT,
                       "feature_order": feature_order, "predict_sets": predict_sets,
                       "params": params or {}}, f)
        try:
            proc = subprocess.run(
                [sys.executable, "-E", RUNNER, work], cwd=work, env=_child_env(),
                capture_output=True, timeout=timeout_s,
            )
        except subprocess.TimeoutExpired as e:
            raise TimeoutError(f"BYO model exceeded its {timeout_s}s sandbox timeout") from e
        out = os.path.join(work, "out")

        vpath = os.path.join(out, "violations.json")
        if os.path.exists(vpath):
            with open(vpath, encoding="utf-8") as f:
                viol = json.load(f)
            if viol:
                raise SandboxViolation(
                    "the BYO model violated the sandbox: "
                    + "; ".join(f"{v['kind']}: {v['detail']}" for v in viol[:5])
                )
        epath = os.path.join(out, "error.json")
        if os.path.exists(epath):
            with open(epath, encoding="utf-8") as f:
                err = json.load(f)
            if err["type"] == "UndeclaredFeature":
                raise UndeclaredFeatureRead(err["message"])
            raise ByoModelError(f"{err['type']}: {err['message']}\n{err['trace']}")
        if proc.returncode != 0:
            raise ByoModelError(
                f"sandbox child exited {proc.returncode}: {proc.stderr.decode('utf-8', 'replace')[-1500:]}"
            )
        with np.load(os.path.join(out, "preds.npz"), allow_pickle=False) as z:
            preds = {k: z[k] for k in z.files}
        model_files: dict[str, bytes] = {}
        mdir = pathlib.Path(out) / "model"
        if mdir.exists():
            for p in sorted(mdir.rglob("*")):
                if p.is_file():
                    model_files[p.relative_to(mdir).as_posix()] = p.read_bytes()
        return preds, model_files
    finally:
        _cleanup(work)
        _cleanup(snap)


# --------------------------------------------------------------------------- #
# Training
# --------------------------------------------------------------------------- #
def _byo_cfg(definition: dict) -> dict:
    cfg = definition.get("byo") or {}
    if not cfg.get("entry") or ":" not in cfg["entry"]:
        raise failures.DataError("definition.byo.entry must be 'module:Class'")
    if not cfg.get("snapshot_dir") or not os.path.isdir(cfg["snapshot_dir"]):
        raise failures.DataError("definition.byo.snapshot_dir must be a readable directory")
    return cfg


def _declared_features(cfg: dict, df: pd.DataFrame, label_col: str) -> list[str]:
    cols = cfg.get("features")
    if cols:
        missing = [c for c in cols if c not in df.columns]
        if missing:
            raise failures.DataError(f"declared features missing from the dataset: {missing}")
        return list(cols)
    return list(engine.feature_frame(df, label_col).columns)


def _feature_order(cfg: dict, features: list[str]) -> tuple[list[str], str]:
    hashes = cfg.get("feature_hashes")
    if hashes:
        missing = [c for c in features if c not in hashes]
        if missing:
            raise failures.DataError(f"feature_hashes does not cover: {missing}")
        return [hashes[c] for c in features], "feature_hash"
    return list(features), "column_name"


def _fold_slices(definition: dict, cfg: dict, n: int) -> list[tuple[int, slice, slice, slice]]:
    raw = cfg.get("folds") or ([definition["_wf_fold"]] if definition.get("_wf_fold") else None)
    if raw:
        from .schemas import FoldSpec

        out = []
        for r in raw:
            f = r if isinstance(r, FoldSpec) else FoldSpec(**r)
            out.append((int(f.index), slice(f.train_start, f.train_end), slice(f.cal_start, f.cal_end),
                        slice(f.test_start, f.test_end)))
        return out
    embargo = int(definition.get("_embargo_bars", 0) or 0)
    tr, va, te = engine.split_indices(n, embargo=embargo)
    return [(0, tr, va, te)]


def _scaled(X: np.ndarray, fit_rows: slice):
    mean, std = engine.fit_scaler(X[fit_rows])
    return engine.apply_scaler(X, mean, std).astype(np.float32), mean, std


def _label_metrics(objective: str, prefix: str, y, pred) -> dict:
    fn = engine.classification_metrics if objective == "classification" else engine.regression_metrics
    return fn(prefix, y, pred)


def train(definition: dict, df: pd.DataFrame, emit) -> tuple[bytes, dict]:
    """Fit a BYO model per fold plus a final refit, all in the sandbox."""
    cfg = _byo_cfg(definition)
    engine.seed_everything(definition)
    objective = engine.resolve_objective(definition)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    features = _declared_features(cfg, df, label_col)
    order, order_src = _feature_order(cfg, features)
    timeout_s = float(cfg.get("timeout_s", DEFAULT_TIMEOUT_S))
    entry = cfg["entry"]
    params = cfg.get("params") or {}

    files = _snapshot_files(cfg["snapshot_dir"])
    if not files:
        raise failures.DataError("the code snapshot is empty")
    chash = code_hash(files)
    if cfg.get("code_hash") and cfg["code_hash"] != chash:
        raise failures.DataError(
            f"snapshot hash {chash} does not match the submitted code_hash {cfg['code_hash']}"
        )

    X_raw = df[features].to_numpy(dtype=np.float64)
    y_cont = df[label_col].to_numpy(dtype=np.float64)
    w_all = engine.sample_weights(df)
    n = len(X_raw)
    folds = _fold_slices(definition, cfg, n)

    def targets(tr: slice):
        y, sigma = engine._apply_label_transforms(y_cont, tr, definition)
        if objective == "classification":
            y = (y > engine.direction_threshold(definition)).astype(np.float64)
        return y.astype(np.float32), sigma

    metrics_parts: list[dict] = []
    fold_records: list[dict] = []
    fold_models: list[dict] = []
    for k, (index, tr, va, te) in enumerate(folds):
        emit("fitting_fold", 100.0 * k / (len(folds) + 1), {"fold": index})
        Xs, _, _ = _scaled(X_raw, tr)
        y, _ = targets(tr)
        arrays = {"X_train": Xs[tr], "y_train": y[tr], "X_val": Xs[va], "X_test": Xs[te]}
        if w_all is not None:
            arrays["w_train"] = w_all[tr].astype(np.float64)
        preds, mfiles = run_sandboxed("fit_predict", entry, files, order, arrays, ["val", "test"],
                                      params, timeout_s=timeout_s)
        part = {**_label_metrics(objective, f"fold{index}_val", y[va], preds["val"]),
                **_label_metrics(objective, f"fold{index}_test", y[te], preds["test"])}
        metrics_parts.append(part)
        fold_records.append({"index": index, "n_train": int(len(Xs[tr])), "n_val": int(len(Xs[va])),
                             "n_test": int(len(Xs[te])), "model_files": sorted(mfiles)})
        fold_models.append({"index": index, "files": mfiles})

    # The final refit: every row any fold used to fit or calibrate.
    fit_lo = min(f[1].start or 0 for f in folds)
    fit_hi = max(f[2].stop if f[2].stop is not None else f[1].stop for f in folds)
    refit = slice(fit_lo, fit_hi)
    emit("refit", 100.0 * len(folds) / (len(folds) + 1), {})
    Xs, mean, std = _scaled(X_raw, refit)
    y, sigma = targets(refit)
    arrays = {"X_train": Xs[refit], "y_train": y[refit]}
    if w_all is not None:
        arrays["w_train"] = w_all[refit].astype(np.float64)
    last_te = folds[-1][3]
    arrays["X_test"] = Xs[last_te]
    _, final_files = run_sandboxed("fit_predict", entry, files, order, arrays, ["test"], params,
                                   timeout_s=timeout_s)

    metrics: dict = {}
    for part in metrics_parts:
        metrics.update(part)
    # Headline numbers are the first fold's, matching the other adapters' val_/test_ keys.
    first = folds[0][0]
    for k, v in list(metrics.items()):
        for split in ("val", "test"):
            pre = f"fold{first}_{split}_"
            if k.startswith(pre):
                metrics[f"{split}_{k[len(pre):]}"] = v
    metrics = engine.assemble_metrics(
        objective, [metrics], framework_version=f"python-{sys.version_info.major}.{sys.version_info.minor}",
        n_train=fold_records[0]["n_train"], n_val=fold_records[0]["n_val"], n_test=fold_records[0]["n_test"],
        extra={"family": "byo", "code_hash": chash, "entry": entry, "folds": fold_records,
               "n_refit_rows": int(refit.stop - refit.start), "feature_order_source": order_src,
               "sandbox": {"network": False, "snapshot": "read_only", "scratch_only_writes": True}},
    )

    inner = pickle.dumps({"entry": entry, "params": params, "snapshot": files, "code_hash": chash,
                          "final": final_files, "folds": fold_models, "sigma": sigma})
    bundle = engine.wrap_bundle(
        inner, framework="byo", objective=objective, feature_order=order,
        scaler={"mean": mean.tolist(), "std": std.tolist()}, arch=entry,
        extra={"family": "byo", "code_hash": chash, "entry": entry, "feature_order_source": order_src,
               "feature_columns": features, "fold_indices": [f["index"] for f in fold_records]},
    )
    return bundle, metrics


def predict_bundle(bundle: bytes, feature_rows, timeout_s: float = DEFAULT_TIMEOUT_S) -> np.ndarray:
    """Score rows with a BYO bundle's final model, in the sandbox.

    `feature_rows` columns are the bundle's `feature_columns` (the training column
    names). Refuses if the stored snapshot no longer hashes to `code_hash`.
    """
    from . import windows

    header, raw = windows.read_bundle(bundle)
    if header.get("framework") != "byo":
        raise failures.DataError("not a BYO bundle")
    payload = pickle.loads(raw)
    if code_hash(payload["snapshot"]) != header["extra"]["code_hash"]:
        raise failures.DataError("the bundle's code snapshot does not match its recorded code_hash")
    df = feature_rows if isinstance(feature_rows, pd.DataFrame) else pd.DataFrame(list(feature_rows))
    cols = header["extra"]["feature_columns"]
    missing = [c for c in cols if c not in df.columns]
    if missing:
        raise failures.DataError("missing required feature(s): " + ", ".join(missing))
    X = df[cols].to_numpy(dtype=np.float64)
    sc = header["scaler"]
    X = engine.apply_scaler(X, np.asarray(sc["mean"], float), np.asarray(sc["std"], float)).astype(np.float32)
    preds, _ = run_sandboxed("predict", payload["entry"], payload["snapshot"], header["feature_order"],
                             {"X_all": X}, ["all"], payload.get("params"), model_in=payload["final"],
                             timeout_s=timeout_s)
    return preds["all"]
