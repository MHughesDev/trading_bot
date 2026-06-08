## Adding a new chart page

1. Import `TradingChart` and `CHART_CONFIG` from `charts`.
2. Build a `TradingChart` with a default symbol and timeframe, then chain `.add_indicator(...)` calls for any overlays or sub-pane studies.
3. Embed the result with `st.components.v1.html(chart.render_html(), height=CHART_CONFIG["size"]["height"])`.
4. Use Streamlit widgets (`st.text_input`, `st.selectbox`) on the same page to let users pick a different symbol or timeframe — each widget change triggers a page rerun, which re-renders the chart with fresh data.
5. The same `TradingChart` setup can be reused on any Streamlit page to keep styling and indicator behaviour identical.
6. For a simple default embed (SMA 20, EMA 50, Bollinger Bands, VWAP) call `render_chart_html(symbol, timeframe)` directly.

## How it works

`render_html()` returns a **self-contained HTML string** that:

- Loads the lightweight-charts JS library from the unpkg CDN (v4.2.0).
- Bakes all OHLCV data and computed indicator series as JSON directly into the `<script>` block.
- Renders a candlestick chart (main pane) with optional overlay series (SMA, EMA, BB, VWAP).
- Appends sub-pane charts below (volume, RSI, MACD) whose time scales stay synced with the main chart.

No Python package from `lightweight-charts-python` is installed or imported. The chart rendering is pure JS.

## Limitations

- **Static after render.** The chart is fully interactive (scroll, zoom, crosshair) but symbol and timeframe selection live in Streamlit widgets on the page, not inside the chart. Changing a widget triggers a Streamlit rerun which fetches fresh data and re-renders the HTML.
- **Python callbacks not used.** `lightweight-charts-python`'s `topbar.switcher` / `events.search` callback model requires a persistent Python event loop, which is incompatible with `components.html()` embedding. This constraint is fundamental to Streamlit's architecture.
- **CDN dependency.** The chart requires network access to `unpkg.com` on first load. In an air-gapped environment, host the `lightweight-charts.standalone.production.js` bundle locally and update `_CDN_URL` in `trading_chart.py`.

## Supported indicators

| Type    | Name              | Key to pass              | Notable params                    |
|---------|-------------------|--------------------------|-----------------------------------|
| Overlay | SMA               | `"sma"`                  | `length` (default 20)             |
| Overlay | EMA               | `"ema"`                  | `length` (default 20)             |
| Overlay | Bollinger Bands   | `"bollinger_bands"`      | `length` (default 20), `std_dev`  |
| Overlay | VWAP              | `"vwap"`                 | —                                 |
| Sub-pane| RSI               | `"rsi"`                  | `length` (default 14)             |
| Sub-pane| MACD              | `"macd"`                 | `fast`, `slow`, `signal`          |
| Sub-pane| Volume            | added by default         | —                                 |

Indicator maths uses pure pandas (rolling means, EWM). If `TA-Lib` is installed it is used automatically as a faster alternative.
