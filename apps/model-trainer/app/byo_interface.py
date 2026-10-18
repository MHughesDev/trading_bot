"""The bring-your-own-model interface (Set N, N-2.9; FEAT-006 section 5).

Kept to numpy + pandas on purpose: the sandbox child imports this module and
nothing heavier, and `apps/tbot` re-exports `ModelInterface` as
`tbot.models.ModelInterface` so user code writes
``from tbot.models import ModelInterface``.
"""

from __future__ import annotations

from abc import ABC, abstractmethod

import numpy as np
import pandas as pd


class UndeclaredFeature(KeyError):
    """The model read a column that is not in the bundle's `feature_order`."""


class StrictFrame(pd.DataFrame):
    """A frame holding only the declared features; reading any other raises
    `UndeclaredFeature` by name rather than pandas' bare `KeyError`."""

    @property
    def _constructor(self):
        return pd.DataFrame

    def __getitem__(self, key):
        names = None
        if isinstance(key, str):
            names = [key]
        elif isinstance(key, (list, tuple)) and all(isinstance(k, str) for k in key):
            names = list(key)
        if names is not None:
            bad = [k for k in names if k not in self.columns]
            if bad:
                raise UndeclaredFeature(
                    f"feature(s) {bad} are not in feature_order {list(self.columns)}"
                )
        return super().__getitem__(key)


def strict_frame(df: pd.DataFrame) -> StrictFrame:
    return StrictFrame(df)


class ModelInterface(ABC):
    """What a submitted model implements. One instance is fitted per fold, plus a
    final refit; each is saved with `save` and restored with `load`.

    * `fit(X, y, sample_weight=None)`: X is a DataFrame whose columns are the
      declared `feature_order` (feature_hash values), oldest row first.
    * `predict(X)`: one value per row: the point forecast for a regression
      objective, the probability of class 1 for classification.
    * `save(path)`: write the fitted state into the directory `path`
      (already created, inside the sandbox scratch space).
    * `load(path)`: classmethod; rebuild a fitted instance from that directory.
    """

    @abstractmethod
    def fit(self, X: pd.DataFrame, y: np.ndarray, sample_weight: np.ndarray | None = None) -> None: ...

    @abstractmethod
    def predict(self, X: pd.DataFrame) -> np.ndarray: ...

    @abstractmethod
    def save(self, path: str) -> None: ...

    @classmethod
    @abstractmethod
    def load(cls, path: str) -> "ModelInterface": ...
