"""Reader for the trainer's self-describing artifact bundle.

Mirrors the writer in the model-trainer's ``engine.wrap_bundle``. Kept dependency
-free and duplicated (rather than shared) because trainer and inference are
separate services. Format:

    MAGIC(8) | header_len(u32 LE) | header_json(utf-8) | inner_artifact_bytes
"""

import json
import struct

MAGIC = b"TBNDL001"


def read_bundle(data: bytes):
    """Return ``(header, inner_bytes)``.

    For legacy/bare artifacts (no magic prefix) returns ``(None, data)`` so the
    caller can fall back to the pre-bundle behavior.
    """
    if not isinstance(data, (bytes, bytearray)) or len(data) < 12:
        return None, data
    if bytes(data[:8]) != MAGIC:
        return None, data
    try:
        hlen = struct.unpack("<I", bytes(data[8:12]))[0]
        header = json.loads(bytes(data[12 : 12 + hlen]).decode("utf-8"))
    except Exception:  # noqa: BLE001
        return None, data
    inner = bytes(data[12 + hlen :])
    return header, inner
