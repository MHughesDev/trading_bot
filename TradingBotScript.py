import requests
import numpy as np
from datetime import datetime, timezone, timedelta
import time
import talib
import pandas as pd

# API Keys for paper trading
API_KEY = "PKP8DTXB6HUI4KB8592J"
API_SECRET_KEY = "2ngUrKMyUvt00M732LQPBtBcqyPg6WiUP9NwITpA"

BASE_URL = "https://paper-api.alpaca.markets/v2"
HEADERS = {
    "APCA-API-KEY-ID": API_KEY,
    "APCA-API-SECRET-KEY": API_SECRET_KEY
}

def place_order(symbol, balance, holdings, side):
    """Place a buy or sell order."""
    endpoint = f"{BASE_URL}/orders"
    amount = round(balance * 0.05, 2) if side == "buy" else round(holdings * 0.5, 2)
    payload = {
        "symbol": symbol,
        "notional": amount,
        "side": side,
        "type": "market",
        "time_in_force": "gtc"
    }

    response = requests.post(endpoint, headers=HEADERS, json=payload)
    if response.status_code == 200:
        print(f"{side.upper()} order placed: {symbol} ${amount}")
        return response.json()
    else:
        print(f"Error placing {side.upper()} order: {response.content}")
        return None

def fetch_crypto_data(symbol, intervals):
    """Fetch market data for a crypto asset."""
    base_url = "https://data.alpaca.markets/v1beta3/crypto/us/bars"
    end_date = datetime.now(timezone.utc)
    start_date = end_date - timedelta(minutes=intervals)
    params = {
        "symbols": symbol,
        "timeframe": "1Min",  # Using 1-minute timeframe
        "start": start_date.isoformat(),
        "end": end_date.isoformat(),
    }
    
    try:
        response = requests.get(base_url, headers=HEADERS, params=params)
        response.raise_for_status()
        data = response.json()
        if "bars" in data and symbol in data["bars"]:
            df = pd.DataFrame(data["bars"][symbol])
            df = df.rename(columns={'c': 'close', 'o': 'open', 'h': 'high', 'l': 'low', 'v': 'volume'})
            return compute_indicators(df)
        else:
            print(f"No data returned for symbol: {symbol}")
            return pd.DataFrame()
    except requests.exceptions.RequestException as e:
        print(f"Error fetching market data: {e}")
        return pd.DataFrame()

def compute_indicators(data):
    """Compute technical indicators."""
    data['EMA_3'] = talib.EMA(data['close'], timeperiod=3)
    data['EMA_5'] = talib.EMA(data['close'], timeperiod=5)
    data['RSI'] = talib.RSI(data['close'], timeperiod=14)
    return data

def fetch_account_balance():
    """Fetch account balance."""
    endpoint = f"{BASE_URL}/account"
    response = requests.get(endpoint, headers=HEADERS)
    if response.status_code == 200:
        account_data = response.json()
        return float(account_data.get("cash", 0))
    else:
        print(f"Error fetching account balance: {response.content}")
        return 0

def fetch_holdings(symbol):
    """Fetch current holdings of a crypto asset."""
    endpoint = f"{BASE_URL}/positions"
    response = requests.get(endpoint, headers=HEADERS)
    if response.status_code == 200:
        positions = response.json()
        for position in positions:
            if position['symbol'] == symbol:
                return float(position['qty'])
        return 0
    else:
        print(f"Error fetching holdings: {response.content}")
        return 0

def trade_signal(data):
    """Generate scalping trade signals based on multiple strategies."""
    if not isinstance(data, pd.DataFrame) or data.empty or len(data) < 100:
        return "None"

    # Calculate Technical Indicators
    data['EMA_50'] = talib.EMA(data['close'], timeperiod=50)
    data['EMA_100'] = talib.EMA(data['close'], timeperiod=100)
    data['Stochastic_K'], data['Stochastic_D'] = talib.STOCH(
        data['high'], data['low'], data['close'], 
        fastk_period=14, slowk_period=3, slowk_matype=0, 
        slowd_period=3, slowd_matype=0
    )
    data['ATR'] = talib.ATR(data['high'], data['low'], data['close'], timeperiod=14)
    
    # Conditions for Long and Short Trades
    buy_signal = (
        data['EMA_50'].iloc[-1] > data['EMA_100'].iloc[-1] and
        data['close'].iloc[-1] > data['EMA_50'].iloc[-1] and
        data['Stochastic_K'].iloc[-1] > 20
    )
    sell_signal = (
        data['EMA_50'].iloc[-1] < data['EMA_100'].iloc[-1] and
        data['close'].iloc[-1] < data['EMA_50'].iloc[-1] and
        data['Stochastic_K'].iloc[-1] < 80
    )
    
    # Price Action at Support/Resistance Levels (Simplified with ATR)
    resistance_level = data['high'].max()  # Simplified; Use pivot points or Fibonacci retracement for precision
    support_level = data['low'].min()
    breakout_buy = data['close'].iloc[-1] > resistance_level + data['ATR'].iloc[-1]
    breakout_sell = data['close'].iloc[-1] < support_level - data['ATR'].iloc[-1]

    # Combining All Scalping Strategies
    if buy_signal or breakout_buy:
        return "BUY"
    elif sell_signal or breakout_sell:
        return "SELL"
    
    return "None"


def run_trading_bot():
    """Main trading bot logic."""
    symbol = "BTC/USD"
    while True:
        market_data = fetch_crypto_data(symbol, intervals=200)
        if not market_data.empty:
            balance = fetch_account_balance()
            holdings = fetch_holdings(symbol)
            signal = trade_signal(market_data)
            if signal == "BUY":
                place_order(symbol, balance, holdings, "buy")
            elif signal == "SELL":
                place_order(symbol, balance, holdings, "sell")
            else:
                print(f"No trade signal for {symbol}")
        
        time.sleep(60)  # Wait for 1 minute

if __name__ == "__main__":
    run_trading_bot()
