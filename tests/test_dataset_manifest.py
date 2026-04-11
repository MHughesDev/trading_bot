from forecaster_model.data.dataset_manifest import build_manifest_from_arrays, compute_content_hash
import numpy as np


def test_manifest_and_hash():
    c = np.cumsum(np.random.default_rng(0).normal(0, 1, size=200)) + 100
    m = build_manifest_from_arrays(c, source_id="test")
    assert m.train_end_index < m.val_end_index
    assert compute_content_hash(c) == m.content_sha256
