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
        format: str | None = None,
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
        if format:
            params["format"] = format
        if self.project_id:
            params["project_id"] = self.project_id
        return self._request("GET", "/api/data/bars", params=params)

    def _data(self, path: str, params: dict[str, Any]) -> dict[str, Any]:
        """A project-scoped data read. ``None`` parameters are dropped."""
        params = {k: v for k, v in params.items() if v is not None}
        if self.project_id:
            params["project_id"] = self.project_id
        return self._request("GET", path, params=params)

    def search(self, q: str, asset_class: str | None = None, limit: int | None = None) -> dict[str, Any]:
        """Ranked instrument search over what this project may see."""
        return self._data("/api/data/search", {"q": q, "asset_class": asset_class, "limit": limit})

    def trades(self, instrument: str, start: str, end: str, as_of: str | None = None,
               limit: int | None = None, format: str | None = None) -> dict[str, Any]:
        return self._data("/api/data/trades", {"instrument": instrument, "start": start, "end": end,
                                               "as_of": as_of, "limit": limit, "format": format})

    def open_interest(self, venue: str, instrument: str, start: str, end: str,
                      as_of: str | None = None, format: str | None = None) -> dict[str, Any]:
        return self._data("/api/data/open_interest", {"venue": venue, "instrument": instrument, "from": start,
                                                      "to": end, "as_of": as_of, "format": format})

    def option_chain(self, underlying: str, as_of: str | None = None, venue: str | None = None,
                     format: str | None = None) -> dict[str, Any]:
        return self._data("/api/data/option_chain", {"underlying": underlying, "as_of": as_of,
                                                     "venue": venue, "format": format})

    def fundamentals(self, cik: int, fields: str | None = None, as_of: str | None = None,
                     format: str | None = None) -> dict[str, Any]:
        return self._data("/api/data/fundamentals", {"cik": cik, "fields": fields, "as_of": as_of,
                                                     "format": format})

    def features(self, name: str, instrument: str, start: str, end: str, tf: str = "1m",
                 as_of: str | None = None, warmup: int | None = None,
                 format: str | None = None) -> dict[str, Any]:
        return self._data("/api/data/features", {"name": name, "instrument": instrument, "start": start,
                                                 "end": end, "tf": tf, "as_of": as_of, "warmup": warmup,
                                                 "format": format})

    # -- memory (AGENT-003 section 5, AGENT-007) ------------------------------

    def memory(self, bucket: str, subject: str | None = None) -> dict[str, Any]:
        """Entries in one memory bucket (findings, negative_results, ...).

        An unknown bucket is a named refusal, not an empty list: those mean opposite
        things, and the first one invites you to stop looking.
        """
        params = {"subject": subject} if subject else {}
        return self._request("GET", f"/api/memory/{bucket}", params=params)

    def prefer(self, scope: str, statement: str) -> dict[str, Any]:
        """Record a preference the user stated, in their words. The one writable bucket."""
        return self._request("POST", "/api/memory/preferences", json={"scope": scope, "statement": statement})

    def forget(self, bucket: str, entry_id: str) -> dict[str, Any] | None:
        """Archive an entry. Nothing a trial ticket cites is ever erased (it is refused)."""
        return self._request("DELETE", f"/api/memory/{bucket}/{entry_id}")

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

    # ---- skills (Set R-a): the registry's agent-facing half -------------------------
    # The agent can search, propose, check a proposal and vote. Admission, promotion
    # and quarantine are `skills.admit` routes that no agent token can call, so there
    # is deliberately no method here for them.

    def skills_glossary(self, project: str | None = None) -> dict[str, Any]:
        """Every skill this caller can see, and which of them a session carries resident."""
        return self._request("GET", "/api/skills/glossary", params={"project": project} if project else {})

    def skills_search(self, query: str, project: str | None = None, limit: int = 10) -> dict[str, Any]:
        """Full-text search over whole skill bodies. The reply says `mode`: lexical or dense."""
        params: dict[str, Any] = {"q": query, "limit": limit}
        if project:
            params["project"] = project
        return self._request("GET", "/api/skills/search", params=params)

    def skills_show(self, skill_id: str) -> dict[str, Any]:
        return self._request("GET", f"/api/skills/item/{skill_id}")

    def skills_propose(self, project_id: str, name: str, category: str, reason: str, source_ref: str,
                       files: dict[str, str], sources: list[str] | None = None) -> dict[str, Any]:
        """Propose a skill. It must cite the run it came from; the platform verifies and admits."""
        return self._request("POST", "/api/skills/proposals", json={
            "project_id": project_id, "name": name, "category": category, "reason": reason,
            "source_ref": source_ref, "sources": sources or [], "files": files,
        })

    def skills_proposals(self, status: str | None = None) -> dict[str, Any]:
        return self._request("GET", "/api/skills/proposals", params={"status": status} if status else {})

    def skills_rate(self, skill_id: str, session_id: str, vote: int) -> Any:
        return self._request("POST", f"/api/skills/{skill_id}/vote", json={"session_id": session_id, "vote": vote})

    # ---- knowledge model (Set R-b): questions only --------------------------------------
    # Recording what a move did, what a study measured or what a trial scored is the
    # platform's job (`knowledge.write`, held by no agent), so there is no method for it.

    def move_check(self, candidate: dict[str, Any]) -> dict[str, Any]:
        """Has this shape of structural move a record of failure? Warns, or blocks and cites it."""
        return self._request("POST", "/api/knowledge/moves/veto", json={"candidate": candidate})

    def analogs(self, instrument_id: int, venue_id: int, k: int = 5, as_of: str | None = None) -> dict[str, Any]:
        """Nearest past windows by explicit fingerprint; the same episode is never one of them."""
        params: dict[str, Any] = {"instrument_id": instrument_id, "venue_id": venue_id, "k": k}
        if as_of:
            params["as_of"] = as_of
        return self._request("GET", "/api/knowledge/analogs", params=params)

    def regime_profile(self, instrument: str, asset_class: str) -> dict[str, Any]:
        return self._request("GET", "/api/knowledge/regimes/profile",
                             params={"instrument": instrument, "asset_class": asset_class})

    def regime_prior(self, instrument: str, asset_class: str, regime: str | None = None) -> dict[str, Any]:
        params: dict[str, Any] = {"instrument": instrument, "asset_class": asset_class}
        if regime:
            params["regime"] = regime
        return self._request("GET", "/api/knowledge/regimes/prior", params=params)

    def compare_to_random(self, campaign_id: str) -> dict[str, Any]:
        """A campaign against its matched random-search arm; refuses without a completed arm."""
        return self._request("GET", f"/api/knowledge/campaigns/{campaign_id}/compare-to-random")

    def model_calibrate(self, model_id: str, version: int, method: str = "aci_cusum",
                        gamma: float | None = None) -> dict[str, Any]:
        """Adaptive conformal calibration of a served version (N-4.1): runs ACI (or ACI with
        CUSUM resets) over its realised forecasts and records the result as a wrapper version.
        Refuses, with a reason, when fewer than 50 forecasts have been realised."""
        body: dict[str, Any] = {"method": method}
        if gamma is not None:
            body["gamma"] = gamma
        return self._request("POST", f"/api/models/{model_id}/versions/{int(version)}/calibrate", json=body)

    def hypothesis_source(self, hypothesis_id: str, source: str, instrument: str, window_start: str,
                          window_end: str, evidence_ids: list[str] | None = None) -> dict[str, Any]:
        return self._request("POST", "/api/knowledge/hypothesis-sources", json={
            "hypothesis_id": hypothesis_id, "source": source, "instrument": instrument,
            "window_start": window_start, "window_end": window_end, "evidence_ids": evidence_ids or [],
        })
