import hashlib
import os
from urllib.parse import urlparse


class ArtifactStore:
    def __init__(self):
        self.backend = os.environ.get("ARTIFACT_STORE", "fs").lower()
        self.base_path = os.environ.get("ARTIFACT_STORE_PATH", "./artifacts")
        self.bucket = os.environ.get("ARTIFACT_STORE_S3_BUCKET", "")
        self.endpoint_url = os.environ.get("AWS_ENDPOINT_URL") or None
        self.region = os.environ.get("AWS_REGION", "us-east-1")
        self._s3 = None

    # ------------------------------------------------------------------ #
    def _s3_client(self):
        if self._s3 is None:
            import boto3

            self._s3 = boto3.client(
                "s3",
                endpoint_url=self.endpoint_url,
                region_name=self.region,
            )
        return self._s3

    # ------------------------------------------------------------------ #
    def put(self, key: str, data: bytes) -> tuple[str, str, int]:
        sha = hashlib.sha256(data).hexdigest()
        size = len(data)

        if self.backend == "s3":
            client = self._s3_client()
            client.put_object(Bucket=self.bucket, Key=key, Body=data)
            uri = f"s3://{self.bucket}/{key}"
            return uri, sha, size

        # fs backend
        path = os.path.join(self.base_path, key)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
        uri = f"file://{os.path.abspath(path)}"
        return uri, sha, size

    # ------------------------------------------------------------------ #
    def get(self, uri: str) -> bytes:
        if uri.startswith("s3://"):
            parsed = urlparse(uri)
            bucket = parsed.netloc
            key = parsed.path.lstrip("/")
            client = self._s3_client()
            obj = client.get_object(Bucket=bucket, Key=key)
            return obj["Body"].read()

        path = uri
        if path.startswith("file://"):
            path = path[len("file://"):]

        with open(path, "rb") as f:
            return f.read()


_STORE: ArtifactStore | None = None


def get_store() -> ArtifactStore:
    global _STORE
    if _STORE is None:
        _STORE = ArtifactStore()
    return _STORE
