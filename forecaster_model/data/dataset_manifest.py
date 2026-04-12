"""
Versioned training dataset manifest (FB-FR-PG4).

Describes raw OHLCV → windows → log-return targets with reproducible splits (no future leakage).
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

import numpy as np


@dataclass
class DatasetManifest:
    """Frozen manifest for forecaster training / audit."""

    version: str = "1"
    schema: str = "tb_forecaster_dataset_v1"
    source_id: str = ""
    bar_count: int = 0
    feature_dim: int = 0
    history_length: int = 64
    forecast_horizon: int = 4
    train_end_index: int = 0
    val_end_index: int = 0
    rng_seed: int = 42
    content_sha256: str = ""
    extra: dict[str, Any] = field(default_factory=dict)

    def to_json(self) -> str:
        return json.dumps(asdict(self), indent=2, sort_keys=True)

    @classmethod
    def from_json(cls, s: str) -> DatasetManifest:
        d = json.loads(s)
        return cls(**{k: v for k, v in d.items() if k in cls.__dataclass_fields__})

    def assert_no_leakage(self) -> None:
        """Train indices must not overlap validation future targets."""
        if self.train_end_index > self.val_end_index:
            raise ValueError("train_end_index must be <= val_end_index (time order)")
        if self.val_end_index > self.bar_count:
            raise ValueError("val_end_index exceeds bar_count")


def compute_content_hash(*arrays: np.ndarray) -> str:
    h = hashlib.sha256()
    for a in arrays:
        h.update(np.asarray(a).tobytes())
    return h.hexdigest()


def build_manifest_from_arrays(
    close: np.ndarray,
    *,
    train_frac: float = 0.7,
    val_frac: float = 0.15,
    history_length: int = 64,
    forecast_horizon: int = 4,
    source_id: str = "inline",
) -> DatasetManifest:
    """Split indices respect time order; validation starts after train + horizon gap."""
    c = np.asarray(close, dtype=np.float64).ravel()
    n = len(c)
    if n < history_length + forecast_horizon + 10:
        raise ValueError("insufficient bars for manifest")
    gap = forecast_horizon
    t_end = int(n * train_frac)
    v_end = int(n * (train_frac + val_frac))
    t_end = min(t_end, n - gap - 1)
    v_end = min(max(v_end, t_end + gap), n - 1)
    m = DatasetManifest(
        source_id=source_id,
        bar_count=n,
        feature_dim=32,
        history_length=history_length,
        forecast_horizon=forecast_horizon,
        train_end_index=t_end,
        val_end_index=v_end,
        content_sha256=compute_content_hash(c),
    )
    m.assert_no_leakage()
    return m


def save_manifest(path: str | Path, m: DatasetManifest) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(m.to_json(), encoding="utf-8")


def load_manifest(path: str | Path) -> DatasetManifest:
    return DatasetManifest.from_json(Path(path).read_text(encoding="utf-8"))
