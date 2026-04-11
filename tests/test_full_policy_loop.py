from policy_model.integration.full_policy_loop import run_stub_episode
import numpy as np


def test_stub_episode():
    rng = np.random.default_rng(0)
    closes = 100 + np.cumsum(rng.normal(0, 0.1, size=30))
    out = run_stub_episode(closes, max_steps=5)
    assert "return" in out
