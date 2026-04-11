"""FB-PL-P0-01 … FB-PL-P0-04 — policy env protocol, buffer/trainer hooks, offline packets, reward."""

from __future__ import annotations

from datetime import UTC, datetime

import numpy as np

from app.config.settings import AppSettings
from app.contracts.forecast_packet import ForecastPacket
from policy_model.env.environment import TradingPolicyEnvironment
from policy_model.env.replay_env import ReplayPolicyEnvironment
from policy_model.env.reward import one_step_reward
from policy_model.env.runtime_check import assert_trading_policy_environment
from policy_model.integration.offline_forecast_env import OfflineForecastPacketEnvironment
from policy_model.objects import PolicyAction
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.training.actor_critic import ActorCriticTrainer
from policy_model.training.buffer import ReplayBuffer, Transition


def _sample_packets(n: int) -> list[ForecastPacket]:
    base = ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        q_low=[0.0] * 15,
        q_med=[float(i) * 1e-4 for i in range(1, 16)],
        q_high=[0.0] * 15,
        interval_width=[0.01] * 15,
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=1.0,
        ensemble_variance=[0.0] * 15,
        ood_score=0.0,
    )
    return [base] * n


def test_fb_pl_p0_01_replay_satisfies_trading_policy_environment() -> None:
    rng = np.random.default_rng(1)
    closes = 100 + np.cumsum(rng.normal(0, 0.1, size=10))
    env = ReplayPolicyEnvironment(closes, settings=AppSettings())
    assert isinstance(env, TradingPolicyEnvironment)
    assert assert_trading_policy_environment(env) is env


def test_fb_pl_p0_01_offline_satisfies_trading_policy_environment() -> None:
    packets = _sample_packets(5)
    mids = np.linspace(100.0, 101.0, 5)
    env = OfflineForecastPacketEnvironment(packets, mids, settings=AppSettings())
    assert isinstance(env, TradingPolicyEnvironment)


def test_fb_pl_p0_02_replay_buffer_and_trainer() -> None:
    rng = np.random.default_rng(2)
    closes = 100 + np.cumsum(rng.normal(0, 0.1, size=12))
    env = ReplayPolicyEnvironment(closes, settings=AppSettings())
    pol = MultiBranchMLPPolicy(seed=3)
    buf = ReplayBuffer(capacity=100)
    trainer = ActorCriticTrainer(policy=pol)
    obs = env.reset()
    for _ in range(3):
        a = pol.select_action(obs, deterministic=True)
        next_obs, r, done, _ = env.step(a)
        buf.push(Transition(obs, a, r, next_obs, done, {}))
        obs = next_obs
        if done:
            break
    metrics = trainer.update_from_buffer(buf, batch_size=8)
    assert metrics["n"] > 0
    assert "loss" in metrics


def test_fb_pl_p0_03_offline_precomputed_packets_step() -> None:
    packets = _sample_packets(4)
    mids = np.array([100.0, 100.5, 101.0, 101.2])
    env = OfflineForecastPacketEnvironment(packets, mids, settings=AppSettings())
    obs0 = env.reset()
    assert len(obs0.forecast_features) > 0
    a = PolicyAction(target_exposure=0.2)
    _obs1, rew, done, info = env.step(a)
    assert isinstance(rew, float)
    assert "turnover" in info


def test_fb_pl_p0_04_one_step_reward_terms() -> None:
    r = one_step_reward(0.01, 0.5, 0.002, lam_turn=0.1, lam_cost=2.0)
    assert abs(r - (0.01 - 0.1 * 0.5 - 2.0 * 0.002)) < 1e-9
