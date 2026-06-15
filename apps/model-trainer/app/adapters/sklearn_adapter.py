import io

import joblib
import sklearn
from sklearn.ensemble import GradientBoostingClassifier, RandomForestClassifier
from sklearn.linear_model import LogisticRegression

from .base import split_label, train_val_split

_ESTIMATORS = {
    "RandomForestClassifier": RandomForestClassifier,
    "GradientBoostingClassifier": GradientBoostingClassifier,
    "LogisticRegression": LogisticRegression,
}


def _binarize(y):
    nunique = y.nunique(dropna=True)
    if nunique > 2:
        y = (y > y.median()).astype(int)
    else:
        y = y.astype(int)
    return y


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    hp = definition.get("hyperparameters", {}) or {}
    estimator_name = hp.get("estimator", "RandomForestClassifier")
    estimator_cls = _ESTIMATORS.get(estimator_name, RandomForestClassifier)

    X, y = split_label(df)
    y = _binarize(y)

    X_tr, y_tr, X_val, y_val = train_val_split(X, y, frac=0.8)

    # Build estimator with any passthrough kwargs the estimator accepts.
    kwargs = {k: v for k, v in hp.items() if k != "estimator"}
    try:
        model = estimator_cls(**kwargs)
    except TypeError:
        model = estimator_cls()

    emit_progress("fitting", 50.0, {"estimator": estimator_name})
    model.fit(X_tr, y_tr)

    val_accuracy = None
    try:
        if len(X_val) > 0:
            val_accuracy = float(model.score(X_val, y_val))
    except Exception:
        pass

    emit_progress("done", 100.0, {"val_accuracy": val_accuracy})

    buf = io.BytesIO()
    joblib.dump(model, buf)
    artifact_bytes = buf.getvalue()

    metrics = {
        "val_accuracy": val_accuracy,
        "n_train": int(len(X_tr)),
        "n_val": int(len(X_val)),
        "framework_version": sklearn.__version__,
    }
    return artifact_bytes, metrics
