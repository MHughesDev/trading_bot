"""The family registry (N-2.1, MD-06).

`resolve(family, variant)` returns the train callable
`(definition, df, emit_progress) -> (bytes, metrics)` for a registered entry.
Entry points are imported lazily so an unavailable optional dependency in one
family never breaks another.

Legacy requests carry only `framework` (+ `model_kind`, `output.quantile_levels`).
`alias_for` maps those onto `(family, variant)` so every existing framework stays
reachable; `route` is what `worker._route` delegates to.
"""

from __future__ import annotations

import importlib
import json
from functools import lru_cache
from pathlib import Path

from .. import failures

REGISTRY_PATH = Path(__file__).with_name("registry.json")

REQUIRED_KEYS = (
    "family", "variant", "framework", "role", "targets", "outputs", "needs_sequence",
    "uses_gpu", "requires", "selectable_as_candidate", "entrypoint", "description",
    "contamination",
)
ROLES = ("baseline", "candidate", "foundation_baseline")


class RegistryError(failures.TrainerFailure, ValueError):
    """Base of the typed refusals below. Data-class: the request is wrong."""

    terminal = failures.DATA_ERROR


class UnknownFamily(RegistryError):
    pass


class SequenceRequired(RegistryError):
    pass


class NotSelectableAsCandidate(RegistryError):
    pass


@lru_cache(maxsize=1)
def load() -> tuple[dict, ...]:
    raw = json.loads(REGISTRY_PATH.read_text(encoding="utf-8"))
    seen: set[tuple[str, str]] = set()
    for e in raw:
        missing = [k for k in REQUIRED_KEYS if k not in e]
        if missing:
            raise RuntimeError(f"registry entry {e.get('family')}/{e.get('variant')} lacks {missing}")
        if e["role"] not in ROLES:
            raise RuntimeError(f"registry entry {e['family']}/{e['variant']}: bad role {e['role']}")
        if e["role"] == "foundation_baseline" and (e["selectable_as_candidate"] or not e["contamination"]):
            raise RuntimeError("a foundation baseline must be non-selectable and carry contamination")
        key = (e["family"], e["variant"])
        if key in seen:
            raise RuntimeError(f"duplicate registry entry {key}")
        seen.add(key)
    return tuple(raw)


def known() -> list[str]:
    return [f"{e['family']}/{e['variant']}" for e in load()]


def entry(family: str, variant: str) -> dict:
    for e in load():
        if e["family"] == family and e["variant"] == variant:
            return e
    raise UnknownFamily(
        f"unknown model family/variant {family!r}/{variant!r}; known: {', '.join(known())}"
    )


def _import_entrypoint(path: str):
    mod, _, fn = path.partition(":")
    return getattr(importlib.import_module(mod), fn)


def resolve(family: str, variant: str, definition: dict | None = None, *, as_candidate: bool = False):
    """The train callable for `(family, variant)`.

    Refuses: an unknown pair (naming the known list); a `needs_sequence` entry
    when the dataset definition has no `sequence`; and, with `as_candidate`, an
    entry that is not selectable as a candidate (foundation baselines).
    """
    e = entry(family, variant)
    if as_candidate and not e["selectable_as_candidate"]:
        raise NotSelectableAsCandidate(
            f"{family}/{variant} is a {e['role']} and is never selectable as the candidate"
        )
    if e["needs_sequence"]:
        d = definition or {}
        ds = d.get("dataset") or {}
        decl = d.get("declaration") or {}
        seq = (ds.get("sequence") if isinstance(ds, dict) else None) or decl.get("sequence") or d.get("sequence")
        if not seq:
            raise SequenceRequired(
                f"{family}/{variant} needs sequences but the dataset definition has no `sequence`"
            )
    return _import_entrypoint(e["entrypoint"])


def alias_for(framework: str, model_kind: str = "", definition: dict | None = None) -> tuple[str, str]:
    """Legacy `(framework, model_kind, quantile?)` -> `(family, variant)`."""
    fw = (framework or "").lower()
    output = (definition or {}).get("output") or {}
    is_q = bool(output.get("quantile_levels"))
    if fw == "garch":
        return "vol", "garch"
    if is_q and fw in ("lightgbm", "xgboost"):
        return "gbm", f"{fw}_q"
    if is_q and fw == "sklearn":
        return "linear", "sklearn_q"
    if model_kind == "forecaster" and fw not in ("xgboost", "lightgbm", "sklearn", "torch"):
        return "forecaster", "lstm"
    if fw == "torch" and model_kind == "forecaster":
        return "forecaster", "lstm"
    for e in load():
        if e["framework"] == fw:
            return e["family"], e["variant"]
    raise UnknownFamily(f"unsupported framework: {framework}; known: {', '.join(known())}")


def route(framework: str, model_kind: str, definition: dict | None = None):
    """What `worker._route` returns. An explicit `definition["family"]` wins."""
    fam = (definition or {}).get("family")
    if isinstance(fam, dict) and fam.get("family"):
        return resolve(fam["family"], fam.get("variant", ""), definition, as_candidate=True)
    f, v = alias_for(framework, model_kind, definition)
    return resolve(f, v, definition, as_candidate=True)
