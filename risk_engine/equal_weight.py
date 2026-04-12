"""Equal-weight capital allocation across symbols (FB-N3 reference stub).

Use for offline experiments or sizing hints — not wired into the live `RiskEngine` hot path unless you add an explicit integration.
"""

from __future__ import annotations

from decimal import Decimal


def equal_weight_fractions(
    n: int,
    *,
    scale_to_one: bool = True,
) -> list[Decimal]:
    """
    Return ``n`` non-negative fractions that sum to 1 when ``scale_to_one`` is True.

    If ``n < 1``, returns an empty list.
    """
    if n < 1:
        return []
    w = Decimal(1) / Decimal(n)
    out = [w for _ in range(n)]
    if not scale_to_one:
        return out
    # n divisions of 1/n can leave a tiny remainder in Decimal; fix last bucket.
    if n > 1:
        s = sum(out[:-1])
        out[-1] = Decimal(1) - s
    return out
