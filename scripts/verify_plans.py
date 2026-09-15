#!/usr/bin/env python3
"""Repo-wide consistency check over docs/plans/.

Run this after any agent touches a plan file. With many tracks editing plan files
concurrently, the failure mode is not a wrong number in one file -- it is two files
that each look right and disagree with each other.

    python scripts/verify_plans.py

Exit code 0 when clean, 1 when anything is inconsistent. Checks:

  1. Per phase file: task count == header completion count == Acceptance count
     == Traces count == status-label count.
  2. Per set: sum of phase task counts == MASTER's total == MASTER's phase-table sum.
  3. Every relative .md link under docs/plans/ resolves to a file that exists.
  4. No two plan sets claim overlapping migration ranges.
  5. No migration number is claimed by a plan set and also already on disk.
  6. Task IDs are unique within their set.
  7. Every phase file declares a migration range or says it needs none.
"""

import os
import re
import sys
import glob

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLANS = os.path.join(ROOT, "docs", "plans")
SETS = os.path.join(PLANS, "plan-sets")
MIGRATIONS = os.path.join(ROOT, "migrations")

problems = []
notes = []


def problem(msg):
    problems.append(msg)


def parse_ranges(text):
    """Every migration number a 'Migration range:' line claims."""
    claimed = set()
    for line in re.findall(r"\*\*Migration range:\*\*\s*(.+)", text):
        if re.search(r"\bnone\b", line, re.I):
            continue
        for lo, hi in re.findall(r"(\d{4})\s*[-–—]\s*(\d{4})", line):
            claimed.update(range(int(lo), int(hi) + 1))
        # bare numbers not already covered by a range
        for n in re.findall(r"(?<![\d–—-])(\d{4})(?![\d–—-])", line):
            claimed.add(int(n))
    return claimed


def check_links(path, text, base):
    for m in re.finditer(r"\]\(([^)#:]+?\.md)(#[^)]*)?\)", text):
        target = os.path.normpath(os.path.join(base, m.group(1)))
        if not os.path.exists(target):
            problem("%s: broken link -> %s" % (rel(path), m.group(1)))


def rel(p):
    return os.path.relpath(p, ROOT).replace("\\", "/")


# ---------------------------------------------------------------- per set
set_ranges = {}
total_tasks = 0

for setdir in sorted(glob.glob(os.path.join(SETS, "set-*"))):
    setname = os.path.basename(setdir)
    letter = setname.split("-", 1)[1]
    phase_files = sorted(glob.glob(os.path.join(setdir, "phase-*.md")))
    if not phase_files:
        continue

    set_total = 0
    seen_ids = {}
    claimed = set()

    for f in phase_files:
        text = open(f, encoding="utf-8").read()
        check_links(f, text, os.path.dirname(f))

        ids = re.findall(r"^### (%s-\d+\.\d+) " % re.escape(letter), text, re.M)
        if not ids:
            continue  # a phase file with no task headers in this convention

        hdr = re.search(r"\*\*Completion: \d+% \((\d+) / (\d+) tasks\)\*\*", text)
        declared = int(hdr.group(2)) if hdr else -1
        acc = len(re.findall(r"^\*\*Acceptance:\*\*", text, re.M))
        tra = len(re.findall(r"^\*\*Traces:\*\*", text, re.M))
        sta = len(re.findall(r"^`(?:NOT STARTED|IN PROGRESS|DONE|BLOCKED[^`]*)`", text, re.M))

        counts = {"tasks": len(ids), "header": declared, "acceptance": acc,
                  "traces": tra, "status": sta}
        if len(set(counts.values())) != 1:
            problem("%s: %s" % (rel(f), ", ".join("%s=%d" % kv for kv in counts.items())))

        for i in ids:
            if i in seen_ids:
                problem("%s: duplicate task id %s (also in %s)" % (rel(f), i, seen_ids[i]))
            seen_ids[i] = os.path.basename(f)

        if "**Migration range:**" not in text:
            notes.append("%s: no '**Migration range:**' line" % rel(f))
        claimed |= parse_ranges(text)
        set_total += len(ids)

    master = os.path.join(setdir, "MASTER.md")
    if os.path.exists(master):
        mtext = open(master, encoding="utf-8").read()
        check_links(master, mtext, setdir)
        claimed |= parse_ranges(mtext)

        m = re.search(r"\d+ / (\d+) tasks", mtext)
        if m and set_total:
            if int(m.group(1)) != set_total:
                problem("%s/MASTER.md: says %s tasks, phase files hold %d"
                        % (setname, m.group(1), set_total))
        rows = re.findall(r"^\| (?:\*\*)?\d+(?:\*\*)? \| \[[^\]]+\]\([^)]+\) \| (\d+) \|",
                          mtext, re.M)
        if rows and set_total and sum(int(r) for r in rows) != set_total:
            problem("%s/MASTER.md: phase table sums to %d, phase files hold %d"
                    % (setname, sum(int(r) for r in rows), set_total))

    if claimed:
        set_ranges[setname] = claimed
    total_tasks += set_total
    if set_total:
        print("%-10s %3d tasks across %d phase files%s"
              % (setname, set_total, len(phase_files),
                 "  migrations %04d-%04d" % (min(claimed), max(claimed)) if claimed else ""))

# ------------------------------------------------- cross-set migration collisions
names = sorted(set_ranges)
for i, a in enumerate(names):
    for b in names[i + 1:]:
        overlap = set_ranges[a] & set_ranges[b]
        if overlap:
            problem("migration range collision: %s and %s both claim %s"
                    % (a, b, ", ".join("%04d" % n for n in sorted(overlap))))

on_disk = set()
for f in glob.glob(os.path.join(MIGRATIONS, "*.sql")):
    m = re.match(r"(\d{4})_", os.path.basename(f))
    if m:
        on_disk.add(int(m.group(1)))

for name, claimed in sorted(set_ranges.items()):
    clash = claimed & on_disk
    if clash:
        problem("%s claims migrations already on disk: %s"
                % (name, ", ".join("%04d" % n for n in sorted(clash))))

# ------------------------------------------------------------- top-level docs
for f in glob.glob(os.path.join(PLANS, "*.md")):
    check_links(f, open(f, encoding="utf-8").read(), PLANS)

# ------------------------------------------------------------------- report
print("\ntotal planned tasks across all sets: %d" % total_tasks)
if on_disk:
    print("highest migration on disk: %04d" % max(on_disk))

if notes:
    print("\nnotes (not failures):")
    for n in notes:
        print("   ", n)

if problems:
    print("\nPROBLEMS (%d):" % len(problems))
    for p in problems:
        print("   ", p)
    sys.exit(1)

print("\nno problems")
sys.exit(0)
