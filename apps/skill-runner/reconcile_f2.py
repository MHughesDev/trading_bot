"""The template side of the F2 reconciliation (Set S S-5.14; ADR-0029).

`crates/features` computes `yang_zhang_vol`, `hurst`, `efficiency_ratio` and
`fracdiff`; the analysis templates compute the same four, in numpy, because the
runner has no Rust. Two implementations of one estimator are the thing the
one-engine rule exists to prevent, so something has to put the same bars through
both. This is the Python half: it reads bars on stdin, calls **the templates'
own functions** — the ones `estimate_volatility`, `detect_regime` and
`stationarity` call from `run` — and prints what they return as JSON. The Rust
half (`mcp-server`, `tools/features_f2_reconciliation.rs`) compares.

It runs the templates the way the runner does: the preamble is prepended to the
template body and the result is executed, because templates cannot import it.
Nothing here reimplements an estimator; if it did, the comparison would be
between the engine and a third copy.

    python reconcile_f2.py <d>,<d>... <window>   < bars.json
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent


def _template(name: str) -> dict:
    source = (
        (ROOT / "preamble.py").read_text(encoding="utf-8")
        + "\n"
        + (ROOT / "templates" / "analysis" / f"{name}.py").read_text(encoding="utf-8")
    )
    namespace: dict = {"__name__": "__skill__"}
    exec(compile(source, name, "exec"), namespace)  # noqa: S102 - the runner does the same
    return namespace


def main() -> None:
    orders = [float(d) for d in sys.argv[1].split(",")]
    window = int(sys.argv[2])
    bars = json.load(sys.stdin)

    import numpy as np
    import pandas as pd

    frame = pd.DataFrame({k: bars[k] for k in ("open", "high", "low", "close", "volume")})
    close = np.asarray(bars["close"], dtype=float)

    volatility = _template("estimate_volatility")
    regime = _template("detect_regime")
    stationarity = _template("stationarity")

    ranges = volatility["_range_estimators"](frame)
    out = {
        "n": int(close.size),
        # A variance per bar; the engine reports its square root.
        "yang_zhang_variance": ranges["yang_zhang"],
        "hurst": regime["_hurst"](np.diff(np.log(close))),
        "efficiency_ratio": regime["_efficiency_ratio"](close),
        "fracdiff": {
            repr(d): stationarity["_fracdiff_last"](np.log(close), d, window) for d in orders
        },
    }
    print(json.dumps(out))


if __name__ == "__main__":
    main()
