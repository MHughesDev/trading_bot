"""Platform client (AGENT-002, TB §3.1–3.3).

The agent reaches the platform only through ``/api/*``, with a session token scoped
to one project. Everything that keeps research honest — the data cutoff, trial
counting, budgets — lives on the other side of this boundary, so there is nothing
here that can be talked out of enforcing it.

Two design choices are deliberate and worth keeping:

* **Handles, not bytes.** Large outputs come back as ``art_…`` handles with a
  manifest. An agent that pulled a 200 MB parquet extract into its context would
  learn nothing a manifest does not tell it, and would pay for the whole thing.
* **Waiting is event-driven.** ``jobs.wait`` follows a resumable cursor rather than
  polling in a loop, so a long job costs a handful of requests instead of one per
  second for an hour.
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass, field
from typing import Any, Iterable

import httpx


class TbotError(RuntimeError):
    """A platform refusal, carrying the structured detail the API returned."""

    def __init__(self, status: int, payload: dict[str, Any]):
        self.status = status
        self.code = payload.get("error", "unknown")
        self.message = payload.get("message", "")
        self.fix = payload.get("fix")
        detail = f"{self.code}: {self.message}"
        if self.fix:
            detail += f"\n  fix: {self.fix}"
        super().__init__(detail)


@dataclass
class JobResult:
    job_id: str
    state: str
    summary: str | None = None
    result: dict[str, Any] | None = None
    error: dict[str, Any] | None = None

    @property
    def succeeded(self) -> bool:
        return self.state == "succeeded"

    def one_line(self) -> str:
        """One line per job, which is all `wait` ever prints (JB-06)."""
        if self.succeeded:
            return f"{self.job_id} succeeded — {self.summary or 'no summary'}"
        if self.error:
            code = self.error.get("code", "failed")
            fix = self.error.get("fix")
            return f"{self.job_id} {self.state} — {code}" + (f" ({fix})" if fix else "")
        return f"{self.job_id} {self.state}"


@dataclass
class Tbot:
    """Client bound to one project by its token."""

    base_url: str = field(default_factory=lambda: os.environ.get("TBOT_API_URL", "http://127.0.0.1:7080"))
    token: str = field(default_factory=lambda: os.environ.get("TBOT_TOKEN", ""))
    project_id: str | None = field(default_factory=lambda: os.environ.get("TBOT_PROJECT_ID") or None)
    timeout: float = 60.0

    def __post_init__(self) -> None:
        self._client = httpx.Client(
            base_url=self.base_url.rstrip("/"),
            headers={"Authorization": f"Bearer {self.token}"},
            timeout=self.timeout,
        )

    # ── plumbing ────────────────────────────────────────────────────────────

    def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        response = self._client.request(method, path, **kwargs)
        if response.status_code >= 400:
            try:
                payload = response.json()
            except Exception:
                payload = {"error": "http_error", "message": response.text[:400]}
            raise TbotError(response.status_code, payload)
        if not response.content:
            return None
        return response.json()

    # ── data (TB §3.2) ──────────────────────────────────────────────────────

    def bars(
        self,
        instrument: str,
        start: str,
        end: str,
        tf: str = "1m",
        as_of: str | None = None,
        limit: int | None = None,
    ) -> dict[str, Any]:
        """Bars, clipped to this project's research cutoff.

        The response carries a manifest whose ``cutoff_applied`` says whether the
        window was shortened. That field is the honest half of the clipping: the
        data is silently correct for what the project may see, and the manifest says
        so out loud.
        """
        params: dict[str, Any] = {"instrument": instrument, "tf": tf, "start": start, "end": end}
        if as_of:
            params["as_of"] = as_of
        if limit:
            params["limit"] = limit
        if self.project_id:
            params["project_id"] = self.project_id
        return self._request("GET", "/api/data/bars", params=params)

    def catalog(self) -> dict[str, Any]:
        params = {"project_id": self.project_id} if self.project_id else {}
        return self._request("GET", "/api/data/catalog", params=params)

    def live(self, instrument: str) -> dict[str, Any]:
        """Last mark and staleness. Desk projects only (DA-15)."""
        params = {"project_id": self.project_id} if self.project_id else {}
        return self._request("GET", f"/api/data/live/{instrument}", params=params)

    # ── jobs (TB §3.3) ──────────────────────────────────────────────────────

    def submit(
        self,
        kind: str,
        manifest: dict[str, Any],
        experiment_id: str | None = None,
        priority: int | None = None,
    ) -> dict[str, Any]:
        """Submit work.

        ``deduplicated: true`` in the reply means an identical job already existed
        and **no new trial was counted**. Worth reading rather than ignoring: it is
        the difference between having spent a trial and not.
        """
        body: dict[str, Any] = {"kind": kind, "manifest": manifest}
        if experiment_id:
            body["experiment_id"] = experiment_id
        if priority is not None:
            body["priority"] = priority
        if self.project_id:
            body["project_id"] = self.project_id
        return self._request("POST", "/api/jobs", json=body)

    def job(self, job_id: str) -> JobResult:
        raw = self._request("GET", f"/api/jobs/{job_id}")
        return JobResult(
            job_id=raw["job_id"],
            state=raw["state"],
            summary=raw.get("result_summary"),
            result=raw.get("result"),
            error=raw.get("error"),
        )

    def cancel(self, job_id: str) -> dict[str, Any]:
        return self._request("POST", f"/api/jobs/{job_id}/cancel")

    def wait(
        self,
        job_ids: Iterable[str],
        timeout_s: float = 3600.0,
        poll_s: float = 2.0,
    ) -> list[JobResult]:
        """Block until every job reaches a terminal state.

        Follows the event cursor rather than re-reading each job: one request per
        batch of events instead of one per job per tick. The cursor is what makes a
        dropped connection resumable rather than a restart.
        """
        pending = set(job_ids)
        results: dict[str, JobResult] = {}
        cursor = 0
        deadline = time.monotonic() + timeout_s

        while pending and time.monotonic() < deadline:
            params: dict[str, Any] = {"after_id": cursor, "limit": 500}
            if self.project_id:
                params["project_id"] = self.project_id
            page = self._request("GET", "/api/jobs/events", params=params)
            cursor = page.get("last_id", cursor)

            touched = {
                event["job_id"]
                for event in page.get("events", [])
                if event["job_id"] in pending and event["kind"] == "state"
            }
            # An event may arrive between the submit and the first poll, so always
            # reconcile against the job itself rather than trusting the event body.
            for job_id in touched | (pending if not page.get("events") else set()):
                if job_id not in pending:
                    continue
                result = self.job(job_id)
                if result.state in ("succeeded", "failed", "cancelled"):
                    results[job_id] = result
                    pending.discard(job_id)

            if pending:
                time.sleep(poll_s)

        for job_id in pending:
            results[job_id] = self.job(job_id)
        return [results[j] for j in results]

    # ── artifacts (TB §3.10) ────────────────────────────────────────────────

    def artifact(self, handle: str) -> dict[str, Any]:
        """The manifest and summary — never the bytes."""
        return self._request("GET", f"/api/artifacts/{handle}")

    def download(self, handle: str, path: str) -> str:
        """Fetch an artifact's bytes to a local file.

        Bytes go to disk, not into the conversation. That is the whole point of a
        handle: analysis happens in code against a file.
        """
        params = {"project_id": self.project_id} if self.project_id else {}
        response = self._client.get(f"/api/artifacts/{handle}/content", params=params)
        if response.status_code >= 400:
            raise TbotError(response.status_code, {"error": "download_failed"})
        with open(path, "wb") as handle_file:
            handle_file.write(response.content)
        return path

    # ── projects ────────────────────────────────────────────────────────────

    def projects(self) -> dict[str, Any]:
        return self._request("GET", "/api/projects")

    def project(self, project_id: str | None = None) -> dict[str, Any]:
        target = project_id or self.project_id
        if not target:
            raise TbotError(422, {"error": "no_project", "message": "no project id given or bound"})
        return self._request("GET", f"/api/projects/{target}")
