"""``tbot`` command line (AGENT-002, TB §3).

Output is shaped for a model reading it, which means three rules throughout:

* **One line per thing.** A job is one line, a bars query is a handful. The agent
  pays per token for everything it reads, and a wall of JSON teaches it no more than
  a summary does.
* **Refusals say what to do instead.** A `403` with a `fix` is a usable instruction;
  a bare `403` invites a retry loop.
* **ASCII only.** The container is UTF-8, but this CLI is also run from a developer's
  terminal, and a Windows console in cp1252 raises `UnicodeEncodeError` on an arrow.
  Decorative glyphs buy a model nothing and cost portability.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any

from .client import Tbot, TbotError


def _print_json(value: Any) -> None:
    print(json.dumps(value, indent=2, default=str))


def _cmd_data_bars(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.bars(
        args.instrument, args.start, args.end, tf=args.tf, as_of=args.as_of, limit=args.limit
    )
    manifest = payload.get("manifest", {})
    if args.json:
        _print_json(payload)
        return 0

    # The <=10-line summary of DATA-005 §5.
    window = manifest.get("window", {})
    print(f"instrument   {manifest.get('instrument')} {manifest.get('timeframe')}")
    print(f"window       {window.get('start')} -> {window.get('end')}")
    print(f"as_of        {manifest.get('as_of')}")
    print(f"rows         {manifest.get('rows')}{' (truncated)' if manifest.get('truncated') else ''}")
    print(f"project      {manifest.get('project_kind')} {manifest.get('project_id')}")
    cutoff = manifest.get("cutoff_applied")
    if cutoff:
        # Stated loudly: the window asked for was longer than this project may see.
        print(f"cutoff       CLIPPED to {cutoff} (research holdout)")
    else:
        print("cutoff       none applied")
    return 0


def _cmd_data_catalog(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.catalog()
    if args.json:
        _print_json(payload)
        return 0
    print(f"horizon {payload.get('horizon')}  ({payload.get('project_kind')})")
    for row in payload.get("instruments", []):
        print(f"  {row['instrument_id']:12} {row['timeframe']:4} {row['bars']:>9} bars")
    return 0


def _cmd_data_live(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.live(args.instrument)
    if args.json:
        _print_json(payload)
        return 0
    print(f"{args.instrument} last bar {payload.get('last_bar_time')} "
          f"({payload.get('last_bar_age_s')}s ago)")
    return 0


def _cmd_jobs_submit(client: Tbot, args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest)
    result = client.submit(args.kind, manifest, experiment_id=args.experiment, priority=args.priority)
    if args.json:
        _print_json(result)
        return 0
    if result.get("deduplicated"):
        # Said plainly, because it changes whether a trial was spent.
        print(f"{result['job_id']} already existed (no new trial counted) - {result['state']}")
    else:
        print(f"{result['job_id']} submitted - {result['state']}")
    return 0


def _cmd_jobs_wait(client: Tbot, args: argparse.Namespace) -> int:
    results = client.wait(args.job_ids, timeout_s=args.timeout)
    for result in results:
        print(result.one_line())
    if any(r.state == "failed" for r in results):
        return 1
    if any(r.state not in ("succeeded", "failed", "cancelled") for r in results):
        return 2  # timeout
    return 0


def _cmd_jobs_get(client: Tbot, args: argparse.Namespace) -> int:
    result = client.job(args.job_id)
    if args.json:
        _print_json({"job_id": result.job_id, "state": result.state,
                     "summary": result.summary, "result": result.result, "error": result.error})
        return 0
    print(result.one_line())
    return 0


def _cmd_jobs_cancel(client: Tbot, args: argparse.Namespace) -> int:
    result = client.cancel(args.job_id)
    print(f"{result['job_id']} {result['state']}")
    return 0


def _cmd_artifact_get(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.artifact(args.handle)
    if args.json:
        _print_json(payload)
        return 0
    print(f"{payload['handle']}  {payload['type']}  {payload['size_bytes']} bytes")
    print(f"  produced by {payload.get('producer_job') or 'upload'}")
    print(f"  pinned: {payload.get('pinned')}")
    return 0


def _cmd_artifact_download(client: Tbot, args: argparse.Namespace) -> int:
    path = client.download(args.handle, args.out)
    print(f"{args.handle} -> {path}")
    return 0


def _cmd_projects(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.projects()
    if args.json:
        _print_json(payload)
        return 0
    for project in payload.get("projects", []):
        cutoff = project.get("research_cutoff") or "now (Desk)"
        print(f"  {project['kind']:9} {project['name']:24} cutoff={cutoff}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="tbot", description="Platform client for the research agent")
    parser.add_argument("--json", action="store_true", help="raw JSON instead of the summary")
    sub = parser.add_subparsers(dest="group", required=True)

    data = sub.add_parser("data", help="point-in-time data reads").add_subparsers(dest="cmd", required=True)
    bars = data.add_parser("bars")
    bars.add_argument("instrument")
    bars.add_argument("--start", required=True)
    bars.add_argument("--end", required=True)
    bars.add_argument("--tf", default="1m")
    bars.add_argument("--as-of", dest="as_of")
    bars.add_argument("--limit", type=int)
    bars.set_defaults(func=_cmd_data_bars)

    catalog = data.add_parser("catalog")
    catalog.set_defaults(func=_cmd_data_catalog)

    live = data.add_parser("live")
    live.add_argument("instrument")
    live.set_defaults(func=_cmd_data_live)

    jobs = sub.add_parser("jobs", help="durable work").add_subparsers(dest="cmd", required=True)
    submit = jobs.add_parser("submit")
    submit.add_argument("kind")
    submit.add_argument("manifest", help="JSON manifest")
    submit.add_argument("--experiment")
    submit.add_argument("--priority", type=int)
    submit.set_defaults(func=_cmd_jobs_submit)

    wait = jobs.add_parser("wait")
    wait.add_argument("job_ids", nargs="+")
    wait.add_argument("--timeout", type=float, default=3600.0)
    wait.set_defaults(func=_cmd_jobs_wait)

    get = jobs.add_parser("get")
    get.add_argument("job_id")
    get.set_defaults(func=_cmd_jobs_get)

    cancel = jobs.add_parser("cancel")
    cancel.add_argument("job_id")
    cancel.set_defaults(func=_cmd_jobs_cancel)

    artifacts = sub.add_parser("artifact", help="content-addressed outputs").add_subparsers(
        dest="cmd", required=True
    )
    art_get = artifacts.add_parser("get")
    art_get.add_argument("handle")
    art_get.set_defaults(func=_cmd_artifact_get)

    art_dl = artifacts.add_parser("download")
    art_dl.add_argument("handle")
    art_dl.add_argument("--out", required=True)
    art_dl.set_defaults(func=_cmd_artifact_download)

    projects = sub.add_parser("projects", help="research projects and the Desk")
    projects.set_defaults(func=_cmd_projects, cmd="list")

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    client = Tbot()
    try:
        return args.func(client, args)
    except TbotError as error:
        # Refusals print their fix, because a refusal the agent cannot act on just
        # becomes a retry.
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
