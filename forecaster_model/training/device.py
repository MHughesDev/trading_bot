"""PyTorch device resolution (FB-SPEC-05): CPU default, optional CUDA when available."""

from __future__ import annotations


def resolve_torch_device(device: str | None) -> str:
    """
    Return a device string suitable for `torch.device(...)`.

    - ``None`` or ``\"auto\"`` → ``\"cuda\"`` when CUDA is available, else ``\"cpu\"``.
    - ``\"cpu\"`` → ``\"cpu\"``.
    - ``\"cuda\"`` or ``\"cuda:N\"`` → passed through (caller must have CUDA if used).

    Requires ``torch`` to be importable when resolving ``auto`` (CUDA probe).
    """
    import torch

    raw = (device or "auto").strip()
    if not raw:
        raw = "auto"
    lower = raw.lower()
    if lower == "auto":
        return "cuda" if torch.cuda.is_available() else "cpu"
    if lower == "cpu":
        return "cpu"
    return raw
