"""The sandbox child for bring-your-own models (N-2.9). Run as a script:

    python -E byo_runner.py <job_dir>

`job_dir` is the scratch directory. It holds `job.json`, `data.npz` (written by
the parent), and receives `out/` (results). The child:

  * has no network: an audit hook raises on every `socket.*` event and on process
    creation, and `socket.socket` is replaced;
  * cannot write outside the scratch directory (audit hook on `open`);
  * sees the snapshot directory read-only (files are marked read-only by the parent
    and the hook refuses writes to it);
  * records every refused action in `out/violations.json`, so a model that catches
    the exception still fails the run.

This is a policy sandbox for honest-but-fallible code, not a defence against a
hostile one (`ctypes` can still reach the OS); a hostile-code boundary is the
container's network namespace, recorded as ADR-N-PY2-03.
"""

from __future__ import annotations

import importlib
import json
import os
import sys
import traceback

JOB = os.path.abspath(sys.argv[1])
OUT = os.path.join(JOB, "out")
os.makedirs(OUT, exist_ok=True)
sys.dont_write_bytecode = True

import numpy as np  # noqa: E402  (imported before the hook so their DLLs load)
import pandas as pd  # noqa: E402

with open(os.path.join(JOB, "job.json"), encoding="utf-8") as f:
    spec = json.load(f)

TRAINER_ROOT = spec["trainer_root"]
SNAPSHOT = os.path.abspath(spec["snapshot_dir"])
sys.path.insert(0, TRAINER_ROOT)
from app.byo_interface import ModelInterface, strict_frame  # noqa: E402

VIOLATIONS: list[dict] = []


def _record(kind: str, detail: str):
    VIOLATIONS.append({"kind": kind, "detail": detail})
    with open(os.path.join(OUT, "violations.json"), "w", encoding="utf-8") as fh:
        json.dump(VIOLATIONS, fh)


class SandboxViolation(PermissionError):
    pass


_SCRATCH = os.path.normcase(os.path.realpath(JOB))
_WRITE_FLAGS = os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_APPEND | os.O_TRUNC


def _inside_scratch(path) -> bool:
    try:
        p = os.path.normcase(os.path.realpath(os.fsdecode(path)))
    except Exception:  # noqa: BLE001
        return False
    return p == _SCRATCH or p.startswith(_SCRATCH + os.sep) or p == os.path.normcase(os.devnull)


def _audit(event: str, args):
    if event.startswith("socket.") and event not in ("socket.gethostname",):
        _record("network", f"{event} {args!r}"[:200])
        raise SandboxViolation(f"BYO sandbox: network is disabled ({event})")
    if event in ("subprocess.Popen", "os.system", "os.posix_spawn", "os.spawn", "os.exec", "os.fork", "os.forkpty"):
        _record("process", event)
        raise SandboxViolation(f"BYO sandbox: starting processes is disabled ({event})")
    if event == "open":
        path, mode, flags = args[0], args[1], args[2]
        if path is None or isinstance(path, int):
            return
        writes = False
        if isinstance(mode, str) and any(c in mode for c in "wax+"):
            writes = True
        if isinstance(flags, int) and (flags & _WRITE_FLAGS):
            writes = True
        if writes and not _inside_scratch(path):
            _record("write", f"{os.fsdecode(path)}")
            raise SandboxViolation(f"BYO sandbox: writes are limited to the scratch dir ({os.fsdecode(path)})")
    if event in ("os.remove", "os.rename", "os.mkdir", "os.rmdir", "shutil.rmtree", "os.truncate", "os.chmod"):
        target = args[0] if args else None
        if target is not None and not _inside_scratch(target):
            _record("write", f"{event} {target!r}")
            raise SandboxViolation(f"BYO sandbox: {event} outside the scratch dir")


def _block_socket_module():
    import socket

    class _NoSocket:
        def __init__(self, *a, **k):
            _record("network", "socket.socket()")
            raise SandboxViolation("BYO sandbox: network is disabled (socket.socket)")

    def _no(*a, **k):
        _record("network", "socket helper")
        raise SandboxViolation("BYO sandbox: network is disabled")

    socket.socket = _NoSocket
    socket.create_connection = _no
    socket.getaddrinfo = _no
    socket.gethostbyname = _no


def _frame(npz, key, cols):
    return strict_frame(pd.DataFrame(npz[key], columns=cols))


def main() -> int:
    err: dict | None = None
    try:
        sys.path.insert(0, SNAPSHOT)
        # `from tbot.models import ModelInterface` works inside the sandbox.
        import types

        if "tbot" not in sys.modules:
            tb = types.ModuleType("tbot")
            tm = types.ModuleType("tbot.models")
            tm.ModelInterface = ModelInterface
            tb.models = tm
            sys.modules["tbot"] = tb
            sys.modules["tbot.models"] = tm
        _block_socket_module()
        sys.addaudithook(_audit)

        mod_name, _, cls_name = spec["entry"].partition(":")
        cls = getattr(importlib.import_module(mod_name), cls_name)
        for m in ("fit", "predict", "save", "load"):
            if not callable(getattr(cls, m, None)):
                raise TypeError(f"{spec['entry']} does not implement ModelInterface.{m}")
        cols = spec["feature_order"]
        data = np.load(os.path.join(JOB, "data.npz"), allow_pickle=False)
        params = spec.get("params") or {}
        preds: dict[str, np.ndarray] = {}

        if spec["op"] == "fit_predict":
            model = cls(**params) if params else cls()
            sw = data["w_train"] if "w_train" in data.files else None
            model.fit(_frame(data, "X_train", cols), data["y_train"], sw) if sw is not None else model.fit(
                _frame(data, "X_train", cols), data["y_train"]
            )
            mdir = os.path.join(OUT, "model")
            os.makedirs(mdir, exist_ok=True)
            model.save(mdir)
            model = cls.load(mdir)  # the saved state must round-trip
        else:  # predict
            mdir = os.path.join(JOB, "model_in")
            model = cls.load(mdir)

        for name in spec["predict_sets"]:
            X = _frame(data, f"X_{name}", cols)
            p = np.asarray(model.predict(X), dtype=float).reshape(-1)
            if len(p) != len(X):
                raise ValueError(f"predict returned {len(p)} values for {len(X)} rows")
            preds[name] = p
        np.savez(os.path.join(OUT, "preds.npz"), **preds)
    except BaseException as e:  # noqa: BLE001
        err = {
            "type": type(e).__name__,
            "message": str(e)[:2000],
            "trace": traceback.format_exc()[-3000:],
        }
        with open(os.path.join(OUT, "error.json"), "w", encoding="utf-8") as fh:
            json.dump(err, fh)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
