"""Reusable trading chart module — renders via lightweight-charts JS (CDN).

NOTE: The chart is static HTML rendered by components.html(). Symbol and timeframe changes
require a Streamlit page rerun via the widgets on the page. Python-level callbacks from
lightweight-charts-python (topbar.switcher, events.search) are not used here because they
require a persistent Python event loop that is incompatible with Streamlit's components.html()
embedding model.
"""

from __future__ import annotations

import json
from copy import deepcopy
from typing import Any

import pandas as pd

from charts._helpers import (
    TIMEFRAME_OPTIONS,
    compute_indicator_frames,
    heikin_ashi,
    normalize_symbol,
)
from charts.data_feed import OHLCVDataSource, get_ohlcv
from charts.indicators import overlay_keys, reference_levels, sub_pane_keys

CHART_TYPES: tuple[str, ...] = ("candles", "heikin_ashi", "line")

CHART_CONFIG: dict[str, Any] = {
    "size": {"width": 1_120, "height": 760},
    "toolbox": True,
    "layout": {"background_color": "#0B1220", "text_color": "#E5E7EB", "font_size": 13, "font_family": "Inter"},
    "grid": {"vert_enabled": True, "horz_enabled": True, "color": "rgba(148,163,184,0.08)", "style": "solid"},
    "crosshair": {
        "mode": "normal",
        "vert_visible": True,
        "vert_color": "rgba(148,163,184,0.35)",
        "vert_style": "dotted",
        "horz_visible": True,
        "horz_color": "rgba(148,163,184,0.35)",
        "horz_style": "dotted",
    },
    "legend": {"visible": True, "ohlc": True, "percent": True, "lines": True, "font_size": 11, "font_family": "JetBrains Mono"},
    "candles": {
        "up_color": "#22C55E",
        "down_color": "#EF4444",
        "border_up_color": "#22C55E",
        "border_down_color": "#EF4444",
        "wick_up_color": "#22C55E",
        "wick_down_color": "#EF4444",
    },
    "volume": {"up_color": "rgba(34,197,94,0.45)", "down_color": "rgba(239,68,68,0.45)"},
    "topbar": {"timeframes": TIMEFRAME_OPTIONS, "default_timeframe": "1D", "default_symbol": "BTC-USD"},
    "pane_heights": {"volume": 0.20, "rsi": 0.15, "macd": 0.15},
    "indicator_colors": {
        "SMA": "#38BDF8",
        "EMA": "#F59E0B",
        "BB Upper": "#A78BFA",
        "BB Basis": "#94A3B8",
        "BB Lower": "#A78BFA",
        "VWAP": "#F97316",
        "RSI": "#22D3EE",
        "MACD": "#60A5FA",
        "MACD Signal": "#F472B6",
        "MACD Histogram": "#A3E635",
        "Volume": "#64748B",
    },
}

# Derived from the indicator registry so newly-added studies render without edits here.
_OVERLAY_KEYS = overlay_keys()
_SUB_PANE_KEYS = sub_pane_keys()
# Extra line colours cycled for overlays not in indicator_colors (keeps many overlays legible).
_PALETTE = (
    "#38BDF8", "#F59E0B", "#A78BFA", "#34D399", "#F472B6", "#FB7185",
    "#FACC15", "#2DD4BF", "#C084FC", "#4ADE80", "#FB923C", "#60A5FA",
)
_CDN_URL = (
    "https://unpkg.com/lightweight-charts@4.2.0"
    "/dist/lightweight-charts.standalone.production.js"
)


def _to_unix_seconds(times: pd.Series) -> list[int]:
    """Convert a pandas datetime Series to a list of Unix epoch seconds (integers)."""
    return [int(pd.Timestamp(t).timestamp()) for t in times]


def _ohlcv_to_js(frame: pd.DataFrame) -> list[dict[str, Any]]:
    """Serialise OHLCV rows to [{time, open, high, low, close}, ...]."""
    if frame.empty:
        return []
    ts = _to_unix_seconds(frame["time"])
    result = []
    for t, row in zip(ts, frame.itertuples(index=False)):
        result.append(
            {
                "time": t,
                "open": round(float(row.open), 8),
                "high": round(float(row.high), 8),
                "low": round(float(row.low), 8),
                "close": round(float(row.close), 8),
            }
        )
    return result


def _indicator_to_js(sf: pd.DataFrame, series_name: str) -> list[dict[str, Any]]:
    """Serialise an indicator DataFrame to [{time, value}, ...]."""
    if sf.empty or series_name not in sf.columns:
        return []
    ts = _to_unix_seconds(sf["time"])
    values = sf[series_name].tolist()
    return [{"time": t, "value": round(float(v), 6)} for t, v in zip(ts, values)]


class TradingChart:
    """Reusable embedded trading chart with shared styling and indicator support."""

    def __init__(
        self,
        symbol: str,
        default_timeframe: str = "1D",
        *,
        chart_type: str = "candles",
        data_source: OHLCVDataSource | None = None,
    ) -> None:
        self.symbol = normalize_symbol(symbol)
        self.timeframe = default_timeframe
        self.chart_type = chart_type if chart_type in CHART_TYPES else "candles"
        self.data_source = data_source
        self.indicators: list[dict[str, Any]] = [{"name": "volume", "params": {}}]

    def add_indicator(self, name: str, **params: Any) -> "TradingChart":
        """Enable an overlay or sub-pane indicator; preserved across rerenders."""
        normalized = name.lower().replace(" ", "_")
        if not any(item["name"] == normalized for item in self.indicators):
            self.indicators.append({"name": normalized, "params": deepcopy(params)})
        return self

    def render_html(self) -> str:
        """Return a self-contained HTML string for embedding via st.components.v1.html()."""
        frame = self._get_ohlcv(self.symbol, self.timeframe)

        overlay_data: dict[str, list[dict[str, Any]]] = {}
        sub_pane_data: dict[str, dict[str, list[dict[str, Any]]]] = {}
        sub_order: list[str] = []

        # Indicators always compute on the raw OHLCV; only the main candle display is transformed.
        for item in self.indicators:
            key = item["name"]
            computed = compute_indicator_frames(frame, key, **item["params"])
            if key in _OVERLAY_KEYS:
                for series_name, sf in computed.items():
                    overlay_data[series_name] = _indicator_to_js(sf, series_name)
            elif key in _SUB_PANE_KEYS:
                sub_pane_data[key] = {
                    series_name: _indicator_to_js(sf, series_name)
                    for series_name, sf in computed.items()
                }
                if key not in sub_order:
                    sub_order.append(key)

        main_frame = heikin_ashi(frame) if self.chart_type == "heikin_ashi" else frame
        candle_js = _ohlcv_to_js(main_frame)
        line_js = (
            [{"time": d["time"], "value": d["close"]} for d in candle_js]
            if self.chart_type == "line"
            else []
        )
        ref_levels = {k: list(reference_levels(k)) for k in sub_order if reference_levels(k)}
        return self._build_html(candle_js, line_js, overlay_data, sub_pane_data, sub_order, ref_levels)

    def _build_html(
        self,
        ohlcv_js: list[dict[str, Any]],
        line_js: list[dict[str, Any]],
        overlay_data: dict[str, list[dict[str, Any]]],
        sub_pane_data: dict[str, dict[str, list[dict[str, Any]]]],
        sub_order: list[str],
        ref_levels: dict[str, list[float]],
    ) -> str:
        cfg = CHART_CONFIG
        layout = cfg["layout"]
        candles = cfg["candles"]
        vol_cfg = cfg["volume"]
        grid = cfg["grid"]
        cross = cfg["crosshair"]
        total_h = cfg["size"]["height"]
        pane_h = cfg["pane_heights"]
        colors = cfg["indicator_colors"]

        # Sub-panes in selection order; unknown studies get a default height.
        active_sub = list(sub_order)
        sub_px = {k: int(float(pane_h.get(k, 0.15)) * total_h) for k in active_sub}
        main_px = total_h - sum(sub_px.values())

        sub_divs = "\n".join(
            f'  <div id="{k}-pane" style="width:100%;height:{sub_px[k]}px;"></div>'
            for k in active_sub
        )

        sub_js = self._build_sub_pane_js(active_sub, sub_px, vol_cfg, ref_levels)

        return f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<script src="{_CDN_URL}"></script>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ background: {layout['background_color']}; overflow: hidden; font-family: {layout['font_family']}; }}
</style>
</head>
<body>
<div id="main-pane" style="width:100%;height:{main_px}px;"></div>
{sub_divs}
<script>
(function () {{
  var ohlcv = {json.dumps(ohlcv_js)};
  var lineData = {json.dumps(line_js)};
  var overlayData = {json.dumps(overlay_data)};
  var subPaneData = {json.dumps(sub_pane_data)};
  var refLevels = {json.dumps(ref_levels)};
  var indicatorColors = {json.dumps(colors)};
  var palette = {json.dumps(list(_PALETTE))};
  var chartType = {json.dumps(self.chart_type)};

  var _paletteIdx = 0;
  var _assigned = {{}};
  function colorFor(name) {{
    var entries = Object.entries(indicatorColors);
    for (var i = 0; i < entries.length; i++) {{
      if (name.indexOf(entries[i][0]) === 0) return entries[i][1];
    }}
    if (!(name in _assigned)) {{
      _assigned[name] = palette[_paletteIdx % palette.length];
      _paletteIdx += 1;
    }}
    return _assigned[name];
  }}

  // Shared chart options
  var baseOpts = {{
    layout: {{
      background: {{ type: 'solid', color: '{layout['background_color']}' }},
      textColor: '{layout['text_color']}',
      fontSize: {layout['font_size']},
      fontFamily: '{layout['font_family']}',
    }},
    grid: {{
      vertLines: {{ color: '{grid['color']}', visible: {str(grid['vert_enabled']).lower()} }},
      horzLines: {{ color: '{grid['color']}', visible: {str(grid['horz_enabled']).lower()} }},
    }},
    crosshair: {{
      mode: LightweightCharts.CrosshairMode.Normal,
      vertLine: {{ color: '{cross['vert_color']}', visible: {str(cross['vert_visible']).lower()}, labelVisible: true }},
      horzLine: {{ color: '{cross['horz_color']}', visible: {str(cross['horz_visible']).lower()}, labelVisible: true }},
    }},
    timeScale: {{
      timeVisible: true,
      secondsVisible: false,
      borderColor: 'rgba(148,163,184,0.2)',
    }},
    rightPriceScale: {{ borderColor: 'rgba(148,163,184,0.2)' }},
    handleScroll: true,
    handleScale: true,
  }};

  // Build ohlcv lookup by time for volume colouring
  var ohlcvByTime = {{}};
  ohlcv.forEach(function (d) {{ ohlcvByTime[d.time] = d; }});

  // --- Main chart (candlestick / Heikin-Ashi / line + overlays) ---
  var mainChart = LightweightCharts.createChart(
    document.getElementById('main-pane'),
    Object.assign({{}}, baseOpts, {{ height: {main_px} }})
  );

  if (chartType === 'line') {{
    var lineSeries = mainChart.addLineSeries({{
      color: '#38BDF8', lineWidth: 2, priceLineVisible: false, lastValueVisible: true,
    }});
    lineSeries.setData(lineData);
  }} else {{
    var candleSeries = mainChart.addCandlestickSeries({{
      upColor: '{candles['up_color']}',
      downColor: '{candles['down_color']}',
      borderUpColor: '{candles['border_up_color']}',
      borderDownColor: '{candles['border_down_color']}',
      wickUpColor: '{candles['wick_up_color']}',
      wickDownColor: '{candles['wick_down_color']}',
    }});
    candleSeries.setData(ohlcv);
  }}

  Object.entries(overlayData).forEach(function (entry) {{
    var name = entry[0];
    var data = entry[1];
    var line = mainChart.addLineSeries({{
      color: colorFor(name),
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: true,
      title: name,
    }});
    line.setData(data);
  }});

  var allCharts = [mainChart];

{sub_js}

  // Sync visible range across all panes
  allCharts.forEach(function (src, si) {{
    src.timeScale().subscribeVisibleLogicalRangeChange(function (range) {{
      if (range === null) return;
      allCharts.forEach(function (dst, di) {{
        if (di !== si) dst.timeScale().setVisibleLogicalRange(range);
      }});
    }});
  }});

  mainChart.timeScale().fitContent();
}})();
</script>
</body>
</html>"""

    def _build_sub_pane_js(
        self,
        active_sub: list[str],
        sub_px: dict[str, int],
        vol_cfg: dict[str, str],
        ref_levels: dict[str, list[float]],
    ) -> str:
        """Return JS for each active sub-pane chart (generic: any registered oscillator)."""
        blocks: list[str] = []
        for key in active_sub:
            h = sub_px[key]
            var = f"{key}Chart"
            blocks.append(
                f"  // --- {key} pane ---\n"
                f"  var {var} = LightweightCharts.createChart(\n"
                f"    document.getElementById('{key}-pane'),\n"
                f"    Object.assign({{}}, baseOpts, {{ height: {h} }})\n"
                f"  );\n"
                f"  allCharts.push({var});\n"
            )

            if key == "volume":
                up = vol_cfg["up_color"]
                down = vol_cfg["down_color"]
                blocks.append(
                    f"  var volSeries = {var}.addHistogramSeries({{\n"
                    f"    color: colorFor('Volume'),\n"
                    f"    priceFormat: {{ type: 'volume' }},\n"
                    f"  }});\n"
                    f"  var volRaw = (subPaneData['volume'] && subPaneData['volume']['Volume']) || [];\n"
                    f"  var volData = volRaw.map(function (d) {{\n"
                    f"    var bar = ohlcvByTime[d.time];\n"
                    f"    var up = bar && bar.close >= bar.open;\n"
                    f"    return {{ time: d.time, value: d.value, color: up ? '{up}' : '{down}' }};\n"
                    f"  }});\n"
                    f"  volSeries.setData(volData);\n"
                )
                continue

            # Generic oscillator: line per series (histogram for *Histogram* names) + reference levels.
            blocks.append(
                f"  (function () {{\n"
                f"    var pane = subPaneData['{key}'] || {{}};\n"
                f"    var firstKey = Object.keys(pane)[0];\n"
                f"    var base = firstKey ? (pane[firstKey] || []) : [];\n"
                f"    Object.entries(pane).forEach(function (e) {{\n"
                f"      var name = e[0], data = e[1];\n"
                f"      if (name.indexOf('Histogram') !== -1) {{\n"
                f"        var s = {var}.addHistogramSeries({{ color: colorFor(name),\n"
                f"          priceLineVisible: false, lastValueVisible: true, title: name }});\n"
                f"        s.setData(data);\n"
                f"      }} else {{\n"
                f"        var s = {var}.addLineSeries({{ color: colorFor(name), lineWidth: 1,\n"
                f"          priceLineVisible: false, lastValueVisible: true, title: name }});\n"
                f"        s.setData(data);\n"
                f"      }}\n"
                f"    }});\n"
                f"    var refs = {json.dumps(ref_levels.get(key, []))};\n"
                f"    refs.forEach(function (lvl) {{\n"
                f"      var ref = {var}.addLineSeries({{ color: 'rgba(148,163,184,0.4)', lineWidth: 1,\n"
                f"        lineStyle: LightweightCharts.LineStyle.Dashed,\n"
                f"        priceLineVisible: false, lastValueVisible: false }});\n"
                f"      ref.setData(base.map(function (d) {{ return {{ time: d.time, value: lvl }}; }}));\n"
                f"    }});\n"
                f"  }})();\n"
            )

        return "".join(blocks)

    def _get_ohlcv(self, symbol: str, timeframe: str) -> pd.DataFrame:
        source = self.data_source.get_ohlcv if self.data_source else get_ohlcv
        return source(symbol=symbol, timeframe=timeframe)

    def _color_for(self, series_name: str) -> str:
        for prefix, color in CHART_CONFIG["indicator_colors"].items():
            if series_name.startswith(prefix):
                return color
        return "#E5E7EB"


def render_chart_html(symbol: str, timeframe: str) -> str:
    """Return a reusable trading chart HTML fragment for embedding."""
    chart = TradingChart(symbol=symbol, default_timeframe=timeframe)
    chart.add_indicator("sma", length=20)
    chart.add_indicator("ema", length=50)
    chart.add_indicator("bollinger_bands", length=20, std_dev=2.0)
    chart.add_indicator("vwap")
    return chart.render_html()
