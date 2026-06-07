"""Real NumPy policy training: backprop correctness, behavior cloning, actor-critic, critic."""

from __future__ import annotations

import numpy as np

from policy_model.objects import PolicyAction, PolicyObservation
from policy_model.policy.critic import ValueCritic
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.policy.policy_network import PolicyNetwork
from training_pipeline.policy_training.actor_critic import ActorCriticTrainer
from training_pipeline.policy_training.behavior_cloning import (
    BCDataset,
    behavior_cloning_loss,
    train_behavior_cloning,
)
from training_pipeline.policy_training.buffer import ReplayBuffer, Transition


def _obs(rng: np.random.Generator) -> PolicyObservation:
    return PolicyObservation(
        forecast_features=list(rng.normal(0, 1, size=64)),
        portfolio_features=list(rng.normal(0, 1, size=6)),
        execution_features=list(rng.normal(0, 1, size=5)),
        risk_features=list(rng.normal(0, 1, size=4)),
        history_features=None,
    )


def test_forward_with_cache_matches_forward() -> None:
    rng = np.random.default_rng(0)
    pol = MultiBranchMLPPolicy(seed=1)
    obs = _obs(rng)
    a_cache, _ = pol.forward_with_cache(obs)
    a_fwd = float(pol.forward(obs)["action_params"])
    assert abs(a_cache - a_fwd) < 1e-12


def test_backprop_matches_finite_difference_on_w_out() -> None:
    rng = np.random.default_rng(2)
    pol = MultiBranchMLPPolicy(seed=2)
    obs = _obs(rng)
    target = 0.3
    a0, cache = pol.forward_with_cache(obs)
    grads = pol.backward(cache, 2.0 * (a0 - target))  # dL/da for L=(a-target)^2
    i = 7
    eps = 1e-6
    pol._w_out[i, 0] += eps
    a_plus, _ = pol.forward_with_cache(obs)
    pol._w_out[i, 0] -= 2 * eps
    a_minus, _ = pol.forward_with_cache(obs)
    pol._w_out[i, 0] += eps  # restore
    numeric = ((a_plus - target) ** 2 - (a_minus - target) ** 2) / (2 * eps)
    assert abs(grads["w_out"][i, 0] - numeric) < 1e-5


def test_behavior_cloning_reduces_loss() -> None:
    rng = np.random.default_rng(3)
    pol = MultiBranchMLPPolicy(seed=4)
    obss = [_obs(rng) for _ in range(8)]
    experts = list(rng.uniform(-0.8, 0.8, size=8))
    ds = BCDataset(observations=obss, expert_actions=experts)
    before = behavior_cloning_loss(pol, ds)
    out = train_behavior_cloning(pol, ds, epochs=300, lr=0.05)
    after = behavior_cloning_loss(pol, ds)
    assert out["loss"] < out["first_loss"]
    assert after < before
    assert after < 0.05  # fits a small dataset well


def test_critic_converges_to_target() -> None:
    rng = np.random.default_rng(5)
    critic = ValueCritic(seed=5)
    obs = _obs(rng)
    target = 0.2
    first_err = (critic.forward(obs) - target) ** 2
    for _ in range(500):
        critic.update(obs, target, lr=0.1)
    assert abs(critic.forward(obs) - target) < 1e-2
    assert (critic.forward(obs) - target) ** 2 < first_err


def test_actor_critic_update_returns_real_metrics_and_changes_weights() -> None:
    rng = np.random.default_rng(6)
    pol = MultiBranchMLPPolicy(seed=6)
    trainer = ActorCriticTrainer(policy=pol, lr=0.05)
    buf = ReplayBuffer(capacity=64)
    for _ in range(16):
        o = _obs(rng)
        no = _obs(rng)
        act = PolicyAction(target_exposure=float(rng.uniform(-1, 1)))
        buf.push(Transition(o, act, float(rng.uniform(-0.02, 0.02)), no, False, {}))
    w_before = pol._w_out.copy()
    m = trainer.update_from_buffer(buf, batch_size=16)
    assert m["n"] == 16
    assert "actor_loss" in m and "critic_loss" in m and "loss" in m
    # A real gradient step moved the actor weights.
    assert not np.allclose(w_before, pol._w_out)


def test_policy_network_update_dispatch() -> None:
    rng = np.random.default_rng(7)
    net = PolicyNetwork(seed=7)

    # BCDataset path
    ds = BCDataset(observations=[_obs(rng) for _ in range(4)], expert_actions=[0.1, -0.2, 0.3, 0.0])
    out_bc = net.update(ds)
    assert "loss" in out_bc

    # ReplayBuffer path
    buf = ReplayBuffer(capacity=16)
    for _ in range(6):
        buf.push(
            Transition(_obs(rng), PolicyAction(target_exposure=0.1), 0.01, _obs(rng), False, {})
        )
    out_ac = net.update(buf)
    assert out_ac["n"] == 6

    # list[Transition] path
    trans = [
        Transition(_obs(rng), PolicyAction(target_exposure=-0.1), -0.01, _obs(rng), True, {})
        for _ in range(3)
    ]
    out_list = net.update(trans)
    assert out_list["n"] == 3
