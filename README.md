# Alpaca Crypto Bot - Local Environment Setup

## Prerequisites
- Python 3.10+ (3.12 recommended)
- `pip`

## 1) Create and activate a virtual environment
```bash
python3 -m venv .venv
source .venv/bin/activate
```

## 2) Install dependencies
```bash
python -m pip install --upgrade pip
pip install -r requirements.txt
```

## 3) Configure environment variables
Create your local env file from the example:

```bash
cp .env.example .env
```

Then update `.env` with your Alpaca credentials.

## 4) Run the script
```bash
python main.py
```

## Notes
- This project uses `alpaca-py` and `python-dotenv`.
- `.env` is ignored by git, so your credentials stay local.