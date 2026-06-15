import hashlib
from collections import OrderedDict

from .artifact_store import get_store


def _normalize_sha(expected: str | None) -> str:
    if not expected:
        return ""
    expected = expected.strip()
    if expected.startswith("sha256:"):
        expected = expected[len("sha256:"):]
    return expected.lower()


class ArtifactCache:
    """Simple OrderedDict-based LRU cache for artifact bytes.

    Keyed by (model_id, version, sha256). Verifies the sha256 of the
    downloaded bytes against the expected hash (with or without the
    "sha256:" prefix). If the expected hash is empty, verification is
    skipped.
    """

    def __init__(self, max_entries: int = 10):
        self.max_entries = max_entries
        self._cache: "OrderedDict[tuple, bytes]" = OrderedDict()

    def load(self, model_id: str, version: int, sha256: str, uri: str) -> bytes:
        expected = _normalize_sha(sha256)
        key = (model_id, version, expected)

        if key in self._cache:
            self._cache.move_to_end(key)
            return self._cache[key]

        store = get_store()
        data = store.get(uri)

        if expected:
            actual = hashlib.sha256(data).hexdigest()
            if actual != expected:
                raise ValueError(
                    f"artifact hash mismatch: expected {expected}, got {actual}"
                )

        self._cache[key] = data
        self._cache.move_to_end(key)
        while len(self._cache) > self.max_entries:
            self._cache.popitem(last=False)
        return data


CACHE = ArtifactCache(max_entries=10)
