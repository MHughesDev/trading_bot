# Legacy snapshots

## `cryptobot/`

A **verbatim copy** of the files from the historical **`crypto`** Git branch: the standalone Alpaca **CryptoBot** script (`main.py`, `.gitignore`, `README.md`). This is preserved alongside [NautilusMonster V3](../README.md) without merging unrelated histories.

To run from this folder (use a virtualenv and your own `.env` with `API_KEY` / `API_SECRET`):

```bash
cd legacy/cryptobot
pip install alpaca-py python-dotenv
python main.py
```

**Note:** `main.py` prints credential-related values and includes example trading logic—review before use.
