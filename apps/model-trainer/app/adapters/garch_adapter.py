"""GARCH-t volatility adapter (legacy framework alias `garch`).

Since N-2.3 / N-4.2 this is a thin alias onto `app.families.vol` (own MLE, no `arch`
dependency). The bundle carries the fitted recursion state; serving returns the conditional
sigma computed from recent bars, or abstains. The old unconditional-sigma serving fallback
no longer exists.
"""

from ..families import vol


def train(definition: dict, df, emit_progress) -> tuple[bytes, dict]:
    return vol.train_garch(definition, df, emit_progress)
