# AGENTS.md

## Cursor Cloud specific instructions

### Overview

MonsterTrader is a single-file Python cryptocurrency trading bot (`main.py`) that uses the [Alpaca](https://alpaca.markets/) brokerage API. It fetches historical BTCUSD market data and executes trades based on a simple open/close price comparison strategy.

### Dependencies

- Python 3.12+ (pre-installed in the VM)
- Install via `pip install -r requirements.txt` (includes `alpaca-py` and `python-dotenv`)

### Running the application

```
python3 main.py
```

Requires a `.env` file in the workspace root with:
```
API_KEY=<your-alpaca-api-key>
API_SECRET=<your-alpaca-api-secret>
```

These correspond to Alpaca Paper Trading API credentials. The app defaults to paper trading (`PAPER = True` in `main.py`).

### Linting

No linter is configured in the repo. Use `flake8 main.py` for basic style checks.

### Testing

No automated tests exist in the repo. To verify the environment is working, confirm all imports succeed:
```
python3 -c "from alpaca.trading.client import TradingClient; from alpaca.data.historical import CryptoHistoricalDataClient; print('OK')"
```

### Key caveats

- The `.env` file is gitignored. API credentials must be provided via the `API_KEY` and `API_SECRET` environment secrets.
- Without valid Alpaca credentials, `main.py` will fail with a `ValueError` (no credentials) or a `401 Unauthorized` (invalid credentials). This is expected.
- There is no `requirements.txt` in the repo; dependencies are listed only in `README.md`.
