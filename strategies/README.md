# Strategies

Hand-written, backtestable trading strategies in NautilusTrader's native
`Strategy` / `StrategyConfig` shape. Each strategy registers itself in
[`registry.py`](./registry.py) so the control plane and the Asset-page
**Backtest** panel can list it in a dropdown without importing every module
eagerly.

## Backtest engine dependency — fork only

The backtest engine ([`backtesting/nautilus_backtest.py`](../backtesting/nautilus_backtest.py))
is built on **NautilusTrader, sourced exclusively from this platform's own fork**:

> https://github.com/MHughesDev/market_simulator

We do **not** install the PyPI `nautilus_trader` package. The dependency is wired
as a PEP 508 direct git reference in the `backtest_nautilus` optional-dependency
group in [`pyproject.toml`](../pyproject.toml), pointing at the fork. The fork is
currently an unmodified mirror of upstream, so the import package name is still
`nautilus_trader` and all strategy/engine code is written against the documented
upstream API.

### Install

```bash
pip install -e ".[backtest_nautilus]"
```

This pulls and builds NautilusTrader from the fork. Because a source checkout
(unlike the PyPI wheels) compiles the Rust/Cython core, a **Rust toolchain** is
required for the first install:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Listing strategies (`strategies.list_strategies()`) and configuring them in the
UI works **without** this dependency installed — only actually *running* a
backtest needs it. `StrategyDescriptor.load()` and `run_backtest(...)` raise a
clear `ImportError` pointing back here if it's missing.

### Pinning

Once the fork starts to diverge from upstream, pin a tag or commit in
`pyproject.toml` for reproducible builds, e.g.:

```toml
backtest_nautilus = [
  "nautilus_trader @ git+https://github.com/MHughesDev/market_simulator.git@<tag-or-sha>",
]
```

## Adding a strategy

1. Create `strategies/<name>_strategy.py` with a frozen `StrategyConfig`
   subclass (its tunable parameters) and a `Strategy` subclass (its event
   handlers). See [`ema_cross_strategy.py`](./ema_cross_strategy.py) for the
   reference pattern.
2. `register(StrategyDescriptor(...))` it in [`registry.py`](./registry.py),
   describing each tunable parameter with a `StrategyParam` so the UI can render
   the right input. `instrument_id` and `bar_type` are injected by the engine —
   don't list them as params.
3. Add a test (see [`tests/test_nautilus_backtest.py`](../tests/test_nautilus_backtest.py)).
