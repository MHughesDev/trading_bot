# Import necessary libraries
from alpaca.trading.client import TradingClient
from alpaca.data.historical import CryptoHistoricalDataClient
from alpaca.data.requests import CryptoBarsRequest
from alpaca.data.timeframe import TimeFrame
from alpaca.trading.requests import MarketOrderRequest
from alpaca.trading.enums import OrderSide, TimeInForce
from dotenv import load_dotenv
from datetime import datetime, timedelta
import os

# Load environment variables from .env file
load_dotenv()

# Access API key and secret securely
API_KEY = os.getenv("API_KEY")
API_SECRET = os.getenv("API_SECRET")
PAPER = True  # Use False for live trading

print(API_KEY)
print(API_SECRET)

# Initialize Alpaca clients
trading_client = TradingClient(API_KEY, API_SECRET, paper=PAPER)
data_client = CryptoHistoricalDataClient(API_KEY, API_SECRET)

# Function to fetch crypto market data
def fetch_crypto_data(symbol, days=7):
    end_date = datetime.now()
    start_date = end_date - timedelta(days=days)
    
    request_params = CryptoBarsRequest(
        symbol_or_symbols=[symbol],
        timeframe=TimeFrame.Day,
        start=start_date,
        end=end_date
    )
    
    bars = data_client.get_crypto_bars(request_params)
    return bars[symbol]

# Function to place a market order
def place_market_order(symbol, qty, side):
    market_order_data = MarketOrderRequest(
        symbol=symbol,
        qty=qty,
        side=side,
        time_in_force=TimeInForce.GTC  # Good 'til canceled
    )
    
    try:
        order = trading_client.submit_order(order_data=market_order_data)
        print(f"Order placed: {order}")
    except Exception as e:
        print(f"Error placing order: {e}")

# Example usage
if __name__ == "__main__":
    crypto_symbol = "BTCUSD"  # Replace with the cryptocurrency pair you want to analyze/trade

    print("Fetching market data...")
    market_data = fetch_crypto_data(crypto_symbol)
    for bar in market_data:
        print(f"Date: {bar.timestamp}, Open: {bar.open}, Close: {bar.close}")

    # Simple trading logic: Buy if the latest closing price is lower than the opening price
    latest_bar = market_data[-1]
    if latest_bar.close < latest_bar.open:
        print(f"Placing a buy order for {crypto_symbol}...")
        place_market_order(crypto_symbol, qty=0.001, side=OrderSide.BUY)  # Adjust quantity as needed
    else:
        print(f"No trade condition met for {crypto_symbol}.")
        