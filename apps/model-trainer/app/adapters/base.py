from typing import Callable, Protocol
import pandas as pd

EmitProgress = Callable[[str, float, dict], None]


class Adapter(Protocol):
    def train(self, definition: dict, df: "pd.DataFrame", emit_progress: EmitProgress) -> tuple[bytes, dict]:
        ...


def split_label(df):
    """Return (X, y). Label column is "label" if present, else the last column.
    X is the numeric columns excluding the label."""
    if "label" in df.columns:
        label_col = "label"
    else:
        label_col = df.columns[-1]
    y = df[label_col]
    feature_df = df.drop(columns=[label_col])
    X = feature_df.select_dtypes(include=["number"])
    return X, y


def train_val_split(X, y, frac=0.8):
    """Split by row order: first frac is train, rest is validation."""
    n = len(X)
    cut = int(n * frac)
    X_tr = X.iloc[:cut]
    y_tr = y.iloc[:cut]
    X_val = X.iloc[cut:]
    y_val = y.iloc[cut:]
    return X_tr, y_tr, X_val, y_val
