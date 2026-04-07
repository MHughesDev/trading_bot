from __future__ import annotations

from dataclasses import dataclass

import polars as pl


@dataclass(slots=True)
class DatasetBuilder:
    """
    Builds train/eval datasets from feature frames.

    V1 keeps this deterministic and transparent for auditability.
    """

    label_horizon: int = 1

    def with_forward_return_label(self, features: pl.DataFrame) -> pl.DataFrame:
        if features.height == 0:
            return features
        if "close" not in features.columns:
            raise ValueError("features frame requires 'close' column for labeling")
        return (
            features.sort(["symbol", "timestamp"])
            .with_columns(
                (
                    pl.col("close").shift(-self.label_horizon).over("symbol") / pl.col("close")
                    - 1.0
                ).alias(f"label_ret_fwd_{self.label_horizon}")
            )
            .drop_nulls(subset=[f"label_ret_fwd_{self.label_horizon}"])
        )
