from __future__ import annotations

from app.config.settings import AppSettings, load_settings

_BACKFILL_FLOOR = 60  # Kraken /public/OHLC minimum serviceable interval (seconds)


def test_default_bar_interval_meets_kraken_backfill_floor() -> None:
    settings = load_settings()
    assert settings.market_data_bar_interval_seconds >= _BACKFILL_FLOOR, (
        f"market_data_bar_interval_seconds={settings.market_data_bar_interval_seconds} "
        f"is below the Kraken /public/OHLC backfill floor of {_BACKFILL_FLOOR}s"
    )


def test_default_training_granularity_meets_kraken_backfill_floor() -> None:
    settings = load_settings()
    assert settings.training_data_granularity_seconds >= _BACKFILL_FLOOR, (
        f"training_data_granularity_seconds={settings.training_data_granularity_seconds} "
        f"is below the Kraken /public/OHLC backfill floor of {_BACKFILL_FLOOR}s"
    )


def test_training_granularity_is_multiple_of_bar_interval() -> None:
    settings = load_settings()
    bar = settings.market_data_bar_interval_seconds
    train = settings.training_data_granularity_seconds
    assert train % bar == 0, (
        f"training_data_granularity_seconds={train} is not a multiple of "
        f"market_data_bar_interval_seconds={bar}"
    )


def test_bare_appsettings_defaults_also_meet_floor() -> None:
    # Guard against Python-level defaults drifting below the floor independently of the YAML.
    s = AppSettings()
    assert s.market_data_bar_interval_seconds >= _BACKFILL_FLOOR
    assert s.training_data_granularity_seconds >= _BACKFILL_FLOOR
