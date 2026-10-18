# The `layer1_run` job's entry point (Set M M-1.5). The platform concatenates
# research_strategy.py, then this file, and hands the result to the skill runner, so every name
# in research_strategy.py is in scope here and the user's code is exec'd into the same namespace.
#
# Input payload: {"code": str, "params": {...}, "seed": int, "cuts": [close_ns, ...],
#                 "data": {"primary": "<parquet with ts_ns, open, high, low, close, volume>"}}
#
# It returns the full series and one re-run per cut. It does NOT compare them: the comparison
# and the choice of cuts are Rust (`position_series::check_truncation`), so the verdict is not
# something the function being tested can influence.


def _pack(series):
    ts = [int(t) for t in series.index]
    vals = [None if v != v else float(v) for v in series.to_numpy(dtype=float)]
    return {"ts": ts, "target": vals}


def run(payload: dict) -> dict:
    bars = pd.read_parquet(payload["data"]["primary"]).set_index("ts_ns")
    ns = dict(globals())
    ns["__name__"] = "layer1_user"
    try:
        exec(compile(payload["code"], "<layer1>", "exec"), ns)
        fn = ns.get("positions")
        if fn is None or not hasattr(fn, "__research__"):
            return {"refused": True, "reason": "meta.undeclared",
                    "detail": {"message": "define a function named `positions` decorated with @research_strategy"}}
        params = payload.get("params") or {}
        seed = int(payload.get("seed") or 0)
        full, _ = run_positions(fn, bars, params, None, seed)
        meta = fn.__research__
        if meta.timeframe != payload.get("timeframe"):
            return {"refused": True, "reason": "meta.timeframe",
                    "detail": {"message": f"the function declares {meta.timeframe} and the run asked for {payload.get('timeframe')}"}}
        if payload.get("instrument") not in meta.instruments:
            return {"refused": True, "reason": "meta.instrument",
                    "detail": {"message": f"{payload.get('instrument')} is not in the declared instruments {list(meta.instruments)}"}}
        cases = []
        for cut in payload.get("cuts") or []:
            trunc, _ = run_positions(fn, bars, params, int(cut), seed)
            cases.append(dict(cut_ns=int(cut), **_pack(trunc)))
        declared = {k: {"kind": s.kind, "low": s.low, "high": s.high} for k, s in meta.params.items()}
        return {"detail": {
            "full": _pack(full),
            "cases": cases,
            "position_units": meta.position_units,
            "allow_short": meta.allow_short,
            "declared_params": declared,
            "lint": lint_tuned_literals(payload["code"], set(meta.params)),
        }}
    except ResearchStrategyError as e:
        return {"refused": True, "reason": e.code, "detail": {"message": e.message}}
