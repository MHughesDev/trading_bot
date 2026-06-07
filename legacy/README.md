# Legacy snapshots

## `decision_pipeline/`

A **preservation copy** of the retired `decision_engine`, `forecaster_model`, and `policy_model`
packages (the AI decision/forecasting/RL-policy pipeline, FB-AP-XXX). The live runtime no longer
calls into this code — trading decisions are now driven by the strategy-based system
(`strategies/`, `backtesting/nautilus_backtest.py`). This folder exists purely so the
implementation isn't lost: copy it into another repo and delete it from here whenever convenient.
Internal cross-imports between the three packages were rewritten to `legacy.decision_pipeline.*`
so the snapshot still imports cleanly as a unit, but nothing in the runtime depends on it.

`docs/` alongside the code holds the matching human/architecture specs (decision pipeline,
forecaster, policy, and the System-2 LLM agent design) — relocated from `docs/Specs/` and
`docs/Human Provided Specs/` for the same copy-paste-elsewhere-and-delete-later reason. Training
specs moved to `training_pipeline/docs/` instead, since training stays a live (if decoupled)
package.

## `cryptobot/`

A **verbatim copy** of the files from the historical **`crypto`** Git branch: the standalone Alpaca **CryptoBot** script (`main.py`, `.gitignore`, `README.md`). This is preserved alongside [Trading Bot](../README.md) without merging unrelated histories.

To run from this folder (use a virtualenv and your own `.env` with `API_KEY` / `API_SECRET`):

```bash
cd legacy/cryptobot
pip install alpaca-py python-dotenv
python main.py
```

**Note:** `main.py` prints credential-related values and includes example trading logic—review before use.
