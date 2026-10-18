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


def _cmd_data_search(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.search(args.q, asset_class=args.asset_class, limit=args.limit)
    if args.json:
        _print_json(payload)
        return 0
    for row in payload.get("instruments", []):
        print(f"  {row['instrument_id']:16} {row.get('asset_class') or '-':16} {row['bars']:>9} bars")
    return 0


def _print_read(args: argparse.Namespace, payload: dict[str, Any], rows_key: str) -> int:
    """The summary every data read prints: a handful of lines, never the rows."""
    if args.json:
        _print_json(payload)
        return 0
    manifest = payload.get("manifest", {})
    if "handle" in payload:
        print(payload.get("summary", payload["handle"]))
        return 0
    rows = payload.get(rows_key)
    print(f"type         {manifest.get('type')}")
    print(f"rows         {'unmeasured (series not collected here)' if rows is None else manifest.get('rows')}")
    print(f"as_of        {manifest.get('as_of')}")
    print(f"snapshot     {manifest.get('data_snapshot_id')}")
    if manifest.get("cutoff_applied"):
        print(f"cutoff       CLIPPED to {manifest['cutoff_applied']} (research holdout)")
    return 0


def _cmd_data_trades(client: Tbot, args: argparse.Namespace) -> int:
    return _print_read(args, client.trades(args.instrument, args.start, args.end, as_of=args.as_of,
                                           limit=args.limit, format=args.format), "trades")


def _cmd_data_oi(client: Tbot, args: argparse.Namespace) -> int:
    return _print_read(args, client.open_interest(args.venue, args.instrument, args.start, args.end,
                                                  as_of=args.as_of, format=args.format), "open_interest")


def _cmd_data_chain(client: Tbot, args: argparse.Namespace) -> int:
    return _print_read(args, client.option_chain(args.underlying, as_of=args.as_of, venue=args.venue,
                                                 format=args.format), "contracts")


def _cmd_data_fundamentals(client: Tbot, args: argparse.Namespace) -> int:
    return _print_read(args, client.fundamentals(args.cik, fields=args.fields, as_of=args.as_of,
                                                 format=args.format), "facts")


def _cmd_data_features(client: Tbot, args: argparse.Namespace) -> int:
    return _print_read(args, client.features(args.name, args.instrument, args.start, args.end, tf=args.tf,
                                             as_of=args.as_of, warmup=args.warmup, format=args.format),
                       "features")


def _cmd_memory_list(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.memory(args.bucket, subject=args.subject)
    if args.json:
        _print_json(payload)
        return 0
    rows = payload.get("rows", [])
    print(f"{payload.get('bucket')}: {len(rows)} entries"
          + (f" for {payload['subject']}" if payload.get("subject") else ""))
    for row in rows:
        claim = str(row.get("claim") or row.get("statement") or "")[:100]
        print(f"  {row.get('id') or row.get('entry_id')}  {claim}")
    return 0


def _cmd_memory_prefer(client: Tbot, args: argparse.Namespace) -> int:
    result = client.prefer(args.scope, args.statement)
    print(f"recorded {result.get('id') if isinstance(result, dict) else ''}".strip())
    return 0


def _cmd_memory_forget(client: Tbot, args: argparse.Namespace) -> int:
    client.forget(args.bucket, args.entry_id)
    print(f"archived {args.entry_id}")
    return 0


def _cmd_skills_glossary(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.skills_glossary(args.project)
    if args.json:
        _print_json(payload)
        return 0
    resident = set(payload.get("resident", []))
    for row in payload.get("skills", []):
        mark = "*" if row["name"] in resident else " "
        print(f"{mark} {row['name']}  ({row['scope']}, {row['status']})  {row['description']}")
    if payload.get("cut"):
        print(f"cut from the resident set: {', '.join(payload['cut'])}")
    return 0


def _cmd_skills_search(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.skills_search(args.query, args.project, args.limit)
    if args.json:
        _print_json(payload)
        return 0
    print(f"mode: {payload.get('mode')}" + ("" if payload.get("embedder") else "  (no embedder: lexical match over skill bodies)"))
    for row in payload.get("results", []):
        print(f"  {row['name']}  ({row['scope']}, {row['status']})  {row['description']}")
    return 0


def _cmd_skills_show(client: Tbot, args: argparse.Namespace) -> int:
    _print_json(client.skills_show(args.skill_id))
    return 0


def _read_bundle(directory: str) -> dict[str, str]:
    import os
    files: dict[str, str] = {}
    for root, _dirs, names in os.walk(directory):
        for name in names:
            path = os.path.join(root, name)
            rel = os.path.relpath(path, directory).replace(os.sep, "/")
            with open(path, encoding="utf-8") as fh:
                files[rel] = fh.read()
    return files


def _cmd_skills_propose(client: Tbot, args: argparse.Namespace) -> int:
    files = _read_bundle(args.dir)
    if "SKILL.md" not in files:
        print(f"error: {args.dir} has no SKILL.md", file=sys.stderr)
        return 1
    result = client.skills_propose(args.project, args.name, args.category, args.reason, args.source_ref,
                                   files, sources=args.source or None)
    if args.json:
        _print_json(result)
        return 0
    print(f"proposal {result['proposal_id']}: {result['status']} ({result['disposition']['kind']})")
    for v in result.get("verdicts", []):
        print(f"  {'pass' if v['passed'] else 'FAIL'}  {v['step']}  {v['detail'] if not v['passed'] else ''}".rstrip())
    return 0 if result["status"] != "rejected" else 2


def _cmd_skills_proposals(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.skills_proposals(args.status)
    if args.json:
        _print_json(payload)
        return 0
    for row in payload.get("proposals", []):
        print(f"  {row['proposal_id']}  {row['status']:9}  {row['name']}  {row['reason']}")
    return 0


def _cmd_skills_rate(client: Tbot, args: argparse.Namespace) -> int:
    client.skills_rate(args.skill_id, args.session, args.vote)
    print("recorded")
    return 0


def _cmd_knowledge_move_check(client: Tbot, args: argparse.Namespace) -> int:
    import json as _json
    result = client.move_check(_json.loads(args.move))
    if args.json:
        _print_json(result)
        return 0
    veto = result.get("veto", {})
    print(f"{veto.get('verdict')}: {veto.get('why', 'no record of failure')}")
    for ref in veto.get("cites", []):
        print(f"  cites {ref}")
    if result.get("overridden"):
        print("  a person has overridden this veto")
    return 2 if veto.get("verdict") == "block" and not result.get("overridden") else 0


def _cmd_knowledge_analogs(client: Tbot, args: argparse.Namespace) -> int:
    result = client.analogs(args.instrument_id, args.venue_id, args.k, args.as_of)
    if args.json:
        _print_json(result)
        return 0
    print(f"vector: {result['vector']} ({', '.join(result['dimensions'])}); "
          f"same-instrument exclusion zone {result['exclusion_zone_days']} days")
    for a in result["analogs"]:
        print(f"  {a['instrument']}  days {a['start']}..{a['end']}  distance {a['distance']:.3f}")
    return 0


def _cmd_knowledge_regime_profile(client: Tbot, args: argparse.Namespace) -> int:
    _print_json(client.regime_profile(args.instrument, args.asset_class))
    return 0


def _cmd_knowledge_regime_prior(client: Tbot, args: argparse.Namespace) -> int:
    _print_json(client.regime_prior(args.instrument, args.asset_class, args.regime))
    return 0


def _cmd_knowledge_compare(client: Tbot, args: argparse.Namespace) -> int:
    _print_json(client.compare_to_random(args.campaign_id))
    return 0


def _cmd_knowledge_source(client: Tbot, args: argparse.Namespace) -> int:
    result = client.hypothesis_source(args.hypothesis_id, args.source, args.instrument, args.start, args.end,
                                      evidence_ids=args.evidence or None)
    print("repeated trial: " + ("yes (" + str(result["repeats"]) + " earlier)" if result["repeated_trial"] else "no"))
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


def _cmd_models_calibrate(client: Tbot, args: argparse.Namespace) -> int:
    payload = client.model_calibrate(args.model_id, args.version, method=args.method, gamma=args.gamma)
    if args.json:
        _print_json(payload)
        return 0
    cal = payload.get("calibration", {})
    print(f"wrapper      v{payload.get('wrapper_version')} wraps v{payload.get('wraps')} ({cal.get('method')})")
    print(f"coverage     {cal.get('coverage')} (nominal {cal.get('nominal_coverage')})")
    print(f"resets       {len(cal.get('reset_indices') or [])} at {cal.get('reset_indices')}")
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
    bars.add_argument("--format", choices=["json", "parquet"], help="parquet returns an artifact handle")
    bars.set_defaults(func=_cmd_data_bars)

    search = data.add_parser("search")
    search.add_argument("q")
    search.add_argument("--asset-class", dest="asset_class")
    search.add_argument("--limit", type=int)
    search.set_defaults(func=_cmd_data_search)

    def _window(p: argparse.ArgumentParser) -> None:
        p.add_argument("--start", required=True)
        p.add_argument("--end", required=True)
        p.add_argument("--as-of", dest="as_of")
        p.add_argument("--format", choices=["json", "parquet"])

    trades = data.add_parser("trades")
    trades.add_argument("instrument")
    _window(trades)
    trades.add_argument("--limit", type=int)
    trades.set_defaults(func=_cmd_data_trades)

    oi = data.add_parser("open-interest")
    oi.add_argument("venue")
    oi.add_argument("instrument")
    _window(oi)
    oi.set_defaults(func=_cmd_data_oi)

    chain = data.add_parser("option-chain")
    chain.add_argument("underlying")
    chain.add_argument("--venue")
    chain.add_argument("--as-of", dest="as_of")
    chain.add_argument("--format", choices=["json", "parquet"])
    chain.set_defaults(func=_cmd_data_chain)

    fundamentals = data.add_parser("fundamentals")
    fundamentals.add_argument("--cik", type=int, required=True)
    fundamentals.add_argument("--fields", help="comma-separated XBRL tags")
    fundamentals.add_argument("--as-of", dest="as_of")
    fundamentals.add_argument("--format", choices=["json", "parquet"])
    fundamentals.set_defaults(func=_cmd_data_fundamentals)

    feats = data.add_parser("features")
    feats.add_argument("--name", required=True, help="comma-separated feature names")
    feats.add_argument("instrument")
    feats.add_argument("--tf", default="1m")
    feats.add_argument("--warmup", type=int)
    _window(feats)
    feats.set_defaults(func=_cmd_data_features)

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

    memory = sub.add_parser("memory", help="what the research loop has recorded").add_subparsers(
        dest="cmd", required=True
    )
    mem_list = memory.add_parser("list")
    mem_list.add_argument("bucket", help="findings, negative_results, profiles, questions, preferences, ...")
    mem_list.add_argument("--subject")
    mem_list.set_defaults(func=_cmd_memory_list)
    mem_pref = memory.add_parser("prefer", help="record a preference the user stated")
    mem_pref.add_argument("--scope", required=True, help="global, or an instrument/strategy")
    mem_pref.add_argument("--statement", required=True)
    mem_pref.set_defaults(func=_cmd_memory_prefer)
    mem_forget = memory.add_parser("forget", help="archive an entry (never erases)")
    mem_forget.add_argument("bucket")
    mem_forget.add_argument("entry_id")
    mem_forget.set_defaults(func=_cmd_memory_forget)

    skills_p = sub.add_parser("skills", help="reusable procedures: find, propose, rate").add_subparsers(
        dest="cmd", required=True
    )
    sk_gloss = skills_p.add_parser("glossary", help="every skill you can see; * marks the resident ones")
    sk_gloss.add_argument("--project")
    sk_gloss.set_defaults(func=_cmd_skills_glossary)
    sk_search = skills_p.add_parser("search", help="find a skill by what it is for")
    sk_search.add_argument("query")
    sk_search.add_argument("--project")
    sk_search.add_argument("--limit", type=int, default=10)
    sk_search.set_defaults(func=_cmd_skills_search)
    sk_show = skills_p.add_parser("show", help="one skill, its versions and usage")
    sk_show.add_argument("skill_id")
    sk_show.set_defaults(func=_cmd_skills_show)
    sk_prop = skills_p.add_parser("propose", help="propose a skill from a folder holding SKILL.md")
    sk_prop.add_argument("dir", help="the bundle folder")
    sk_prop.add_argument("--project", required=True)
    sk_prop.add_argument("--name", required=True)
    sk_prop.add_argument("--category", required=True)
    sk_prop.add_argument("--reason", required=True, choices=["T1", "T2", "T3", "T4"])
    sk_prop.add_argument("--source-ref", required=True, help="the run it was distilled from, e.g. run:<id>")
    sk_prop.add_argument("--source", action="append", help="a kind of evidence consumed (repeatable)")
    sk_prop.set_defaults(func=_cmd_skills_propose)
    sk_list = skills_p.add_parser("proposals", help="what became of your proposals")
    sk_list.add_argument("--status", choices=["submitted", "verifying", "rejected", "admitted"])
    sk_list.set_defaults(func=_cmd_skills_proposals)
    sk_rate = skills_p.add_parser("rate", help="vote on a skill this session was shown")
    sk_rate.add_argument("skill_id")
    sk_rate.add_argument("--session", required=True)
    sk_rate.add_argument("--vote", type=int, required=True, choices=[-1, 0, 1])
    sk_rate.set_defaults(func=_cmd_skills_rate)

    know = sub.add_parser("knowledge", help="move memory, analogs, regimes, random-arm comparison").add_subparsers(
        dest="cmd", required=True
    )
    kn_move = know.add_parser("move-check", help="has this move a record of failure?")
    kn_move.add_argument("move", help='the move as JSON, e.g. \'{"move": "simplify"}\'')
    kn_move.set_defaults(func=_cmd_knowledge_move_check)
    kn_an = know.add_parser("analogs", help="nearest past windows by explicit fingerprint")
    kn_an.add_argument("--instrument-id", type=int, required=True)
    kn_an.add_argument("--venue-id", type=int, required=True)
    kn_an.add_argument("--k", type=int, default=5)
    kn_an.add_argument("--as-of")
    kn_an.set_defaults(func=_cmd_knowledge_analogs)
    kn_rp = know.add_parser("regime-profile", help="occupancy, transitions and the current regime")
    kn_rp.add_argument("instrument")
    kn_rp.add_argument("--asset-class", required=True)
    kn_rp.set_defaults(func=_cmd_knowledge_regime_profile)
    kn_rr = know.add_parser("regime-prior", help="top strategy families per regime, with observation counts")
    kn_rr.add_argument("instrument")
    kn_rr.add_argument("--asset-class", required=True)
    kn_rr.add_argument("--regime", choices=["low_vol", "mid_vol", "high_vol"])
    kn_rr.set_defaults(func=_cmd_knowledge_regime_prior)
    kn_cr = know.add_parser("compare-to-random", help="a campaign against its matched random-search arm")
    kn_cr.add_argument("campaign_id")
    kn_cr.set_defaults(func=_cmd_knowledge_compare)
    kn_src = know.add_parser("source", help="say where a hypothesis came from (memory:<finding id>)")
    kn_src.add_argument("--hypothesis-id", required=True)
    kn_src.add_argument("--source", required=True)
    kn_src.add_argument("--instrument", required=True)
    kn_src.add_argument("--start", required=True)
    kn_src.add_argument("--end", required=True)
    kn_src.add_argument("--evidence", action="append", help="an experiment, job or artifact id (never a finding)")
    kn_src.set_defaults(func=_cmd_knowledge_source)

    models = sub.add_parser("models", help="model versions").add_subparsers(dest="cmd", required=True)
    mcal = models.add_parser("calibrate", help="adaptive conformal calibration of a version (N-4.1)")
    mcal.add_argument("model_id")
    mcal.add_argument("version", type=int)
    mcal.add_argument("--method", choices=["aci", "aci_cusum"], default="aci_cusum")
    mcal.add_argument("--gamma", type=float)
    mcal.set_defaults(func=_cmd_models_calibrate)

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
