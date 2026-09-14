#!/usr/bin/env python
"""Render one agent run as a readable decision trace.

The event stream in `agent_messages` says WHAT the agent did. Since the decision
record landed it also carries WHY: for every model call, the exact prompt the model
read, the grammar it was decoding under, and the raw reply it produced.

That is a lot of text — a single step is two prompts of several thousand tokens —
so it is useless as a wall and valuable as something you can narrow. Hence the modes:

    agent_trace.py <run|conversation id>              step-by-step summary
    agent_trace.py <id> --step 4                      everything about one step
    agent_trace.py <id> --step 4 --phase select_tool  one decode, in full
    agent_trace.py <id> --grep "list_instruments"     every place a string appears
    agent_trace.py <id> --context 4                   what the model SAW at step 4
    agent_trace.py <id> --dropped                     what compaction removed, and when

Reads Postgres directly through the running container, so it needs no server.
"""
import argparse
import io
import json
import subprocess
import sys
import textwrap

# Windows consoles default to cp1252 and this tool prints box-drawing characters and
# whatever the model wrote. Reconfiguring beats sanitising the output: the trace is
# meant to show exactly what was there.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

PSQL = ["docker", "exec", "trading_bot-postgres-1", "psql", "-U", "trading", "-d", "trading",
        "-t", "-A", "-F", "\x1f", "-c"]


def query(sql):
    out = subprocess.run(PSQL + [sql], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit("psql failed: " + out.stderr.strip())
    rows = []
    for line in out.stdout.replace("\r", "").split("\n"):
        if not line.strip():
            continue
        rows.append(line.split("\x1f"))
    return rows


def resolve_run(ident):
    """Accept a run id or a conversation id; return (run_id, goal, status)."""
    rows = query(
        "SELECT run_id::text, COALESCE(goal,''), status FROM agent_runs "
        "WHERE run_id::text = '%s' OR conversation_id::text = '%s' "
        "ORDER BY created_at DESC LIMIT 1" % (ident, ident))
    if not rows:
        sys.exit("no run found for " + ident)
    return rows[0]


def messages(run_id):
    rows = query(
        "SELECT seq, kind, content_json::text FROM agent_messages "
        "WHERE run_id = '%s' ORDER BY seq" % run_id)
    out = []
    for seq, kind, blob in rows:
        try:
            content = json.loads(blob)
        except Exception:
            content = {"_unparsed": blob[:400]}
        out.append((int(seq), kind, content))
    return out


def short(v, n=110):
    t = v if isinstance(v, str) else json.dumps(v)
    t = " ".join(t.split())
    return t if len(t) <= n else t[: n - 1] + "…"


def step_of(kind, c):
    for key in ("step",):
        if isinstance(c, dict) and key in c:
            return c[key]
    return None


# ── Modes ────────────────────────────────────────────────────────────────────

def summary(msgs):
    """One line per event, grouped by step. The map before the territory."""
    current = None
    for seq, kind, c in msgs:
        st = step_of(kind, c)
        if st is not None and st != current:
            current = st
            print("\n── step %s ──" % st)
        if kind == "user":
            print("  GOAL      %s" % short(c.get("content", "")))
        elif kind == "plan":
            for i, s in enumerate(c.get("steps", [])):
                print("  PLAN  %d.  %s  [%s]" % (i + 1, s.get("goal"), s.get("namespace")))
        elif kind == "exposure":
            line = "  EXPOSE    %s" % ", ".join(c.get("exposed", []))
            if c.get("pinned"):
                line += "   pinned=%s" % ",".join(c["pinned"])
            if c.get("plan_step"):
                line += "   for=%r" % short(c["plan_step"], 40)
            print(line)
        elif kind == "decision":
            print("  DECIDE    %-14s %s -> %s  [%sms, in=%s out=%s]" % (
                c.get("phase"), c.get("tool") or "", short(c.get("raw_reply", ""), 60),
                c.get("usage", {}).get("latency_ms"),
                c.get("usage", {}).get("input_tokens"),
                c.get("usage", {}).get("output_tokens")))
        elif kind == "tool_call":
            print("  CALL      %s(%s)" % (c.get("name"), short(c.get("arguments", {}), 70)))
        elif kind == "tool_result":
            tag = "ERROR" if c.get("is_error") else "ok"
            print("  RESULT    %-5s %s" % (tag, short(c.get("content", ""), 80)))
        elif kind == "validation":
            print("  REJECT    %s  %s" % (c.get("code"), short(c.get("message", ""), 70)))
        elif kind == "policy":
            print("  POLICY    %s %s -> %s" % (c.get("tool"), c.get("risk"), c.get("decision")))
        elif kind == "compaction":
            rm = c.get("removed") or []
            if rm or c.get("dropped") or c.get("truncated"):
                print("  COMPACT   dropped=%s truncated=%s deduped=%s  %s" % (
                    c.get("dropped"), c.get("truncated"), c.get("deduped"),
                    "; ".join("%s %s from %s (%sB)" % (r["how"], r["section"], r["source"], r["bytes"])
                              for r in rm) or ""))
        elif kind == "core_tool":
            print("  CORE      %s  %s" % (c.get("name"), short(c.get("detail", ""), 70)))
        elif kind == "degradation":
            print("  DEGRADED  %s %s" % (c.get("degradation"), short(c, 60)))
        elif kind == "error":
            print("  ERROR     %s" % short(c.get("error", ""), 100))
        elif kind == "canary":
            verdicts = [p.get("verdict") for p in c] if isinstance(c, list) else []
            print("  CANARY    %s" % ", ".join(verdicts))
        elif kind == "status" and any(k in c for k in ("hardware", "tool_calling_effective")):
            print("  SETUP     %s" % short(c, 100))


def one_step(msgs, step, phase=None):
    """Everything that happened in one step, prompts in full."""
    for seq, kind, c in msgs:
        if step_of(kind, c) != step:
            continue
        if kind == "decision":
            if phase and c.get("phase") != phase:
                continue
            print("=" * 78)
            print("DECISION  step %s  phase=%s  tool=%s" % (step, c.get("phase"), c.get("tool")))
            print("=" * 78)
            print("\n--- GRAMMAR (what was reachable) ---")
            print(json.dumps(c.get("schema"), indent=2)[:4000])
            print("\n--- PROMPT (what the model read) ---")
            print(c.get("prompt", ""))
            print("\n--- RAW REPLY ---")
            print(c.get("raw_reply", ""))
            print("\n--- SETTINGS/USAGE ---")
            print(json.dumps({"settings": c.get("settings"), "usage": c.get("usage")}, indent=2))
            print()
        elif not phase:
            print("[%s] %s" % (kind, short(c, 200)))


def context_at(msgs, step):
    """Only the prompt bodies for a step — the fastest way to see what it knew."""
    for seq, kind, c in msgs:
        if kind != "decision" or step_of(kind, c) != step:
            continue
        print("── %s (step %s) ──" % (c.get("phase"), step))
        print(c.get("prompt", ""))
        print()


def grep(msgs, needle):
    """Where does this string appear, and in what role?"""
    low = needle.lower()
    for seq, kind, c in msgs:
        blob = json.dumps(c)
        if low not in blob.lower():
            continue
        where = []
        if kind == "decision":
            for field in ("prompt", "raw_reply", "system"):
                if low in str(c.get(field, "")).lower():
                    where.append(field)
            print("seq %-4s step %-3s decision/%-14s in %s" % (
                seq, c.get("step"), c.get("phase"), ",".join(where) or "schema"))
        else:
            print("seq %-4s %-12s %s" % (seq, kind, short(c, 120)))


def dropped(msgs):
    """What the model was not shown, and when. Often the whole answer."""
    any_rm = False
    for seq, kind, c in msgs:
        if kind != "compaction":
            continue
        for r in c.get("removed") or []:
            any_rm = True
            print("seq %-4s %-9s %-13s from %-28s %6sB" % (
                seq, r["how"], r["section"], r["source"], r["bytes"]))
    if not any_rm:
        print("nothing was dropped or truncated in this run")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ident", help="run id or conversation id")
    ap.add_argument("--step", type=int)
    ap.add_argument("--phase", choices=["plan", "select_tool", "fill_arguments", "native"])
    ap.add_argument("--grep")
    ap.add_argument("--context", type=int, metavar="STEP")
    ap.add_argument("--dropped", action="store_true")
    a = ap.parse_args()

    run_id, goal, status = resolve_run(a.ident)
    msgs = messages(run_id)
    print("run %s  [%s]" % (run_id, status))
    print(textwrap.fill("goal: " + goal, 100, subsequent_indent="      "))
    print("%d events" % len(msgs))

    if a.grep:
        print()
        grep(msgs, a.grep)
    elif a.dropped:
        print()
        dropped(msgs)
    elif a.context is not None:
        print()
        context_at(msgs, a.context)
    elif a.step is not None:
        print()
        one_step(msgs, a.step, a.phase)
    else:
        summary(msgs)


if __name__ == "__main__":
    main()
