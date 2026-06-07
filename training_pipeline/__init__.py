"""Decoupled AI-model training pipelines (FB-AP-XXX).

Everything in this package is **training-time only** and is deliberately **not part of the live
runtime**. Nothing here is imported by the trading hot path or started by the control-plane
process; the former nightly in-process scheduler has been removed. These modules are invoked
on demand (e.g. the per-asset Initialize flow, or a manual/offline training campaign) and are
intended to migrate into a standalone training package/dependency
(see the user's external training-pipeline repo).

Subpackages:
    forecaster_training  — forecaster fitting / distillation / walk-forward (was training_pipeline.forecaster_training)
    policy_training      — RL actor-critic / behavior-cloning / replay buffer (was training_pipeline.policy_training)
    orchestration        — nightly campaign + promotion glue (was the training-only parts of orchestration/)
"""
