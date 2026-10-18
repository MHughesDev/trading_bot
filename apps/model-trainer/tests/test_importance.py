import numpy as np

from app import importance


def _data(n=600, seed=0):
    rng = np.random.default_rng(seed)
    a = rng.normal(size=n)
    x = np.column_stack([a, a + 0.05 * rng.normal(size=n), rng.normal(size=n), rng.normal(size=n)])
    y = a + 0.3 * rng.normal(size=n)
    return x, y


def test_clusters_group_the_near_copy_and_leave_noise_alone():
    x, _ = _data()
    assert importance.correlation_clusters(x) == [[0, 1], [2], [3]]


def test_the_informative_cluster_has_the_largest_increase_and_noise_is_near_zero():
    x, y = _data()
    predict = lambda m: m[:, 0]  # a fitted model that uses the signal
    ranges = [(0, 150), (150, 300), (300, 450), (450, 600)]
    out = importance.permutation_increases(predict, x, y, ranges, [[0, 1], [2], [3]])
    means = [float(np.mean(v)) for v in out["per_fold"]]
    assert means[0] > 10 * max(abs(means[1]), abs(means[2]))
    assert out["folds"] == 4


def test_it_never_chooses_a_row_so_an_empty_range_is_skipped_not_invented():
    x, y = _data(50)
    out = importance.permutation_increases(lambda m: m[:, 0], x, y, [(10, 10)], [[0]])
    assert out["folds"] == 0 and out["per_fold"] == [[]]
