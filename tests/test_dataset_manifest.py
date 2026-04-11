import numpy as np
import pytest

from forecaster_model.data.dataset_manifest import DatasetManifest, build_manifest_from_arrays, compute_content_hash
from forecaster_model.training.walkforward import WalkForwardConfig, walk_forward_indices


def test_manifest_and_hash():
    c = np.cumsum(np.random.default_rng(0).normal(0, 1, size=200)) + 100
    m = build_manifest_from_arrays(c, source_id="test")
    assert m.train_end_index < m.val_end_index
    assert compute_content_hash(c) == m.content_sha256


def test_assert_no_leakage_rejects_bad_order():
    m = DatasetManifest(
        bar_count=100,
        train_end_index=80,
        val_end_index=50,
    )
    with pytest.raises(ValueError, match="train_end_index"):
        m.assert_no_leakage()


def test_walk_forward_train_before_val():
    """Validation ranges must be strictly after training (no lookahead)."""
    cfg = WalkForwardConfig(train_len=20, val_len=10, step=5)
    pairs = walk_forward_indices(100, cfg)
    assert len(pairs) >= 2
    for tr, va in pairs:
        assert tr.stop <= va.start
        assert set(tr).isdisjoint(set(va))
