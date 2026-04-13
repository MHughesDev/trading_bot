<div align="center">

# 🤖 Trading Bot

**A Python playground for AI-assisted crypto trading:** one codebase for **research**, **paper trading**, and **careful live execution** — with **Kraken** for market data and **Alpaca (paper)** / optional **Coinbase (live)** for orders.

[![Python 3.11+](https://img.shields.io/badge/python-3.11+-3776AB?style=flat&logo=python&logoColor=white)](https://www.python.org/downloads/)
[![Code style: Ruff](https://img.shields.io/badge/code%20style-ruff-261230.svg?style=flat)](https://github.com/astral-sh/ruff)

</div>

---

## ⚡ Easy start (do this first)

Right after you clone, these scripts set up a virtualenv, install the app, bring up the local data stack (Docker), and copy **`.env.example` → `.env`** if you do not have one yet.

### 🐧 Linux / macOS

```bash
git clone https://github.com/MHughesDev/trading_bot.git
cd trading_bot
chmod +x setup.sh run.sh    # once
./setup.sh                  # install + infra (runs package-index preflight first)
./run.sh                    # API + supervisor + dashboard
./doctor.sh                 # optional env doctor (full audit readiness checks)
```

### 🪟 Windows

```bat
git clone https://github.com/MHughesDev/trading_bot.git
cd trading_bot
setup.bat
run.bat
doctor.bat                  REM optional env doctor
```

**Tips**

- Want to skip Docker for a run? Set **`NM_SKIP_DOCKER=1`** (venv and pip still run). See [`.env.example`](.env.example).
- Local baseline: use **Python 3.12** for parity with CI (project minimum remains 3.11+).
- Deeper checklist (keys, Docker, preflight): [`docs/READY_TO_RUN.MD`](docs/READY_TO_RUN.MD) · Windows UI notes: [`docs/WINDOWS_OPERATOR_UI.MD`](docs/WINDOWS_OPERATOR_UI.MD).

---

## 🛰️ When the system is *actually watching* an asset for you

Think of it as a little factory line — not every bell and whistle, just the happy path while the live loop is on and a symbol is in scope:

| Step | What happens |
|:---:|:---|
| 📡 | **Kraken** streams trades, ticker, and book updates over the WebSocket. |
| 🧹 | Messages are **normalized** into clean snapshots (ticks, spreads, depth). |
| 📊 | **Bars roll** and the **feature pipeline** turns price action into signals the models understand. |
| 🧠 | The **decision engine** runs the same **`run_decision_tick`** path as replay — so paper and live stay honest with each other. |
| 🛡️ | **Risk + signing** get the last word before anything leaves the building. |
| 📝 | **Orders** go to **Alpaca (paper)** by default, or **Coinbase** when you have configured live mode and keys. |
| 💾 | Along the way, **QuestDB / Redis / Qdrant** (via Docker) back bars, cache, and optional memory — so the app is not reinventing a database in RAM. |

**One-liner vibe:** *market noise in → features → decision → risk → (maybe) trade.*

---

## 🎁 The boring stuff you still want

| | |
|:---:|:---|
| 🔑 | Put secrets in **`.env`** — never commit them. App settings use the **`NM_`** prefix (see [`.env.example`](.env.example)). |
| 📦 | Default execution mode is **paper**; go **live** only when you mean it and keys are set. |
| 🧪 | Dev quickies: `python3 -m ruff check .` and `python3 -m pytest tests/ -q` (after `pip install -e ".[dev]"`). |

**Want the full map?** [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD) · **Repo contract for contributors:** [`AGENTS.md`](AGENTS.md).

---

<div align="center">

*Have fun, measure twice, trade responsibly.*

</div>
