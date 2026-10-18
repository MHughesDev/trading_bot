"""N-2.8: foundation baselines are baselines only and carry contamination."""
import numpy as np
import pytest

from _fam_common import folds, noop_emit, ret_frame
from app.families import baseline as bl
from app.families import foundation, registry


def test_absent_package_gives_typed_unavailable_never_numbers():
    df = ret_frame()
    row = foundation.baseline_row("timesfm", {"declaration": {"window": {"start": "2025-01-01"}}}, df)
    assert row["status"] == "unavailable" and row["scores"] is None and row["series"] is None
    assert row["family"] == "foundation" and row["reason"]
    with pytest.raises(foundation.FoundationUnavailable) as ei:
        foundation.train_ttm({}, df, noop_emit)
    assert ei.value.result["status"] == "unavailable"


def test_contamination_flag_follows_cutoff_vs_window_start():
    df = ret_frame()
    early = foundation.contamination("chronos", {"declaration": {"window": {"start": "2021-01-01", "end": "2025-06-30"}}}, df)
    assert early["model_cutoff"] == "2024-01-01" and early["contaminated"] is True
    assert "2021-01-01" in early["note"] and "2025-06-30" in early["note"]
    late = foundation.contamination("chronos", {"declaration": {"window": {"start": "2025-01-01"}}}, df)
    assert late["contaminated"] is False
    unknown = foundation.contamination("chronos", {}, df)
    assert unknown["contaminated"] is True  # conservative


def test_row_with_a_backend_scores_on_same_folds_and_names_the_window():
    df = ret_frame(signal=0.01)
    foundation.register_backend("chronos", lambda ctx, h: float(np.mean(ctx[-50:])))
    try:
        d = {"declaration": {"window": {"start": "2021-01-01", "end": "2022-01-01"}}}
        f = folds(len(df))
        row = foundation.baseline_row("chronos", d, df, f)
        assert row["status"] == "ok" and row["folds"] == [0, 1, 2] and np.isfinite(row["scores"]["rmse"])
        assert row["contamination"]["contaminated"] is True
        assert "2021-01-01" in row["contamination"]["note"]
        # surfaced beside the other baselines when asked for
        rows, ids = bl.evaluate_baselines({**d, "baseline": {"foundation": ["chronos"]}}, df, f)
        assert rows[-1]["name"] == "chronos" and rows[-1]["contamination"]["model_cutoff"] == "2024-01-01"
        assert ids == [0, 1, 2]
    finally:
        foundation.register_backend("chronos", None)


def test_foundation_is_never_the_candidate():
    for v in foundation.VARIANTS:
        with pytest.raises(registry.NotSelectableAsCandidate):
            registry.resolve("foundation", v, as_candidate=True)
    foundation.register_backend("ttm", lambda c, h: 0.0)
    try:
        with pytest.raises(registry.NotSelectableAsCandidate):
            foundation.train_ttm({}, ret_frame(), noop_emit)
    finally:
        foundation.register_backend("ttm", None)
