"""Walk-forward triple split geometry."""

from orchestration.walkforward_triple import triple_splits


def test_triple_splits_ordering() -> None:
    splits = triple_splits(20, 2)
    assert len(splits) == 2
    for s in splits:
        assert s.train.stop <= s.validation.start
        assert s.validation.stop <= s.test.start
        assert len(s.train) > 0 and len(s.validation) > 0 and len(s.test) > 0
