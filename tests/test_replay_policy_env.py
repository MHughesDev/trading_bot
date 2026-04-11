"""ReplayPolicyEnvironment (FB-PL-PG1)."""

from __future__ import annotations

import numpy as np

from app.config.settings import AppSettings
from policy_model.env.replay_env import ReplayPolicyEnvironment
from policy_model.objects import PolicyAction


def test_replay_env_reset_step() -> None:
    rng = np.random.default_rng(0)
    closes = 100 + np.cumsum(rng.normal(0, 0.1, size=20))
    s = AppSettings()
    env = ReplayPolicyEnvironment(closes, settings=s)
    obs0 = env.reset()
    assert len(obs0.forecast_features) > 0
    a = PolicyAction(target_exposure=0.5)
    obs1, rew, done, info = env.step(a)
    assert isinstance(rew, float)
    assert "turnover" in info
    assert not done or obs1 is not None
