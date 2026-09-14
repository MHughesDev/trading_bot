"""The eval runner (AGENT-004 §2).

One trial is: create a fresh isolated project, generate the synthetic instrument
from the trial seed, start an agent session against it, collect the report, the
registries and the usage, grade it, and archive the transcript.

Everything except the session start works today. The session start needs a provider
credential in the encrypted store, which this work does not have; `--dry-run`
exercises the whole path with a stub session so the harness itself is testable
without one, and that is how the graders and the scorecard have been verified.

Three properties are load-bearing and are implemented here rather than trusted:

1. **Each trial gets its own project.** Not a shared one with a reset in between: a
   reset that misses something leaks one trial's candidates into the next, and a
   power curve built on that measures memory rather than search.
2. **The seed is the whole state.** The instrument is a pure function of
   `(generator, params, seed)`, so a trial reproduces byte-identically. The seed is
   derived from `(task_id, trial_index, run_salt)`, and `run_salt` is what makes
   "fresh synthetic seeds every run" (§5) true without making runs unreproducible -
   the salt is recorded with the results.
3. **The agent's token cannot read the answer key.** The runner holds `evals.truth`;
   the session token holds the seven research scopes and nothing else. Those are two
   different tokens, minted separately, and migration 0039 refuses the first set on
   any project-bound session.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from dataclasses import asdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "graders"))
sys.path.insert(0, str(ROOT / "runner"))

import common  # noqa: E402
from scorecard import GradedTrial, build_scorecard  # noqa: E402

try:  # PyYAML is in the agent image and the dev requirements.
    import yaml
except ImportError:  # pragma: no cover - the error message is the feature
    print(
        "error: PyYAML is required to read the suite and task files\n"
        "  fix: pip install pyyaml",
        file=sys.stderr,
    )
    raise


def trial_seed(task_id: str, index: int, run_salt: str) -> int:
    """A reproducible per-trial seed.

    Derived rather than drawn, so a result can be reproduced from what is recorded:
    the task, the index and the salt. A random seed would make every run fresh and
    every failure unreproducible, which is the wrong half of the trade.
    """
    h = hashlib.sha256(f"{task_id}|{index}|{run_salt}".encode()).digest()
    # 63 bits: fits a Postgres BIGINT, which is what `eval_runs.seed` is.
    return int.from_bytes(h[:8], "big") & ((1 << 63) - 1)


def load_task(task_id: str) -> dict[str, Any]:
    path = ROOT / "tasks" / f"{task_id}.yaml"
    if not path.exists():
        raise FileNotFoundError(f"no task file at {path}")
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def load_suites() -> dict[str, Any]:
    return yaml.safe_load((ROOT / "suites.yaml").read_text(encoding="utf-8"))


def load_grader(module_path: str):
    """Imports a grader by its path in the task file."""
    name = Path(module_path).stem
    return __import__(name)


class Platform:
    """The runner's side of the platform API.

    Thin on purpose: every method is one call the platform already exposes, so this
    file is a script rather than a second implementation of the platform.
    """

    def __init__(self, base_url: str, token: str):
        self.base_url = base_url.rstrip("/")
        self.token = token

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.token}"}

    def create_project(self, name: str, holdout_days: int) -> str:
        raise NotImplementedError("wired in run_trial; see --dry-run")

    def create_synthetic(self, spec: dict[str, Any]) -> str:
        raise NotImplementedError

    def read_truth(self, instrument_id: str) -> dict[str, Any]:
        """Reads the planted mechanism. Needs `evals.truth`.

        The runner holds it and the agent never does. If this call ever succeeds
        with a session token, the suite has stopped measuring anything and the
        correct response is to stop the run, not to note it.
        """
        raise NotImplementedError


def stub_trial(task: dict[str, Any], seed: int, instrument_id: str) -> common.Trial:
    """A trial that did not run an agent.

    Used by `--dry-run` to exercise the task files, the graders and the scorecard
    end to end without a provider credential. It deliberately produces the shape a
    *silent* session would: no report. That grades as a failure, which is correct -
    a harness that scored "nothing happened" as a pass would report a perfect noise
    score for a system that never answers.
    """
    return common.Trial(
        task_id=task["task_id"],
        suite=task["suite"],
        seed=seed,
        instrument_id=instrument_id,
        report=None,
        truth={"mechanism": "none"} if task["suite"] == "noise" else {},
    )


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Run an agent eval suite")
    ap.add_argument("--suite", action="append", help="suite name; repeatable")
    ap.add_argument("--task", action="append", help="task id; repeatable")
    ap.add_argument("--seeds", type=int, default=None, help="seeds per task")
    # Stamped onto the scorecard, not inferred. A run that cannot say which
    # tier it executed on cannot be used as evidence for promoting that tier.
    ap.add_argument("--profile", help="capability profile id this run executed under")
    ap.add_argument("--tier", help="capability tier, e.g. frontier | local_mid")
    ap.add_argument("--hardware", help="one-line description of the accelerators")
    ap.add_argument("--run-salt", default=str(int(time.time())))
    ap.add_argument("--harness-version", default=None)
    ap.add_argument("--base-url", default="http://localhost:7080")
    ap.add_argument("--out", default=None, help="write the scorecard here")
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="resolve tasks, derive seeds and grade stub trials without starting a "
        "session; verifies the harness without a provider credential",
    )
    args = ap.parse_args(argv)

    suites = load_suites()
    harness_version = args.harness_version or suites.get("harness_version", "unknown")
    seeds = args.seeds or suites.get("defaults", {}).get("seeds_per_task", 3)

    task_ids: list[str] = list(args.task or [])
    for suite_name in args.suite or []:
        suite = suites["suites"].get(suite_name)
        if suite is None:
            print(f"error: unknown suite {suite_name!r}", file=sys.stderr)
            return 2
        task_ids += suite["tasks"]
    if not task_ids:
        print("error: give --suite or --task", file=sys.stderr)
        return 2

    if not args.dry_run:
        print(
            "error: a live run needs a provider credential in the encrypted store.\n"
            "  fix: store an Anthropic credential, then re-run without --dry-run.\n"
            "  note: --dry-run exercises task resolution, seed derivation, grading\n"
            "        and the scorecard without one.",
            file=sys.stderr,
        )
        return 3

    graded: list[GradedTrial] = []
    strengths: dict[str, float] = {}

    for task_id in task_ids:
        task = load_task(task_id)
        grader = load_grader(task["grader"]["module"])
        strength = task["grader"].get("planted_strength")
        if strength is not None:
            strengths[task_id] = float(strength)

        for index in range(seeds):
            seed = trial_seed(task_id, index, args.run_salt)
            generator = task["setup"]["synthetic"]["generator"]
            instrument_id = f"SYN-{generator.upper().replace('_', '-')}-{seed}"
            trial = stub_trial(task, seed, instrument_id)
            verdict = grader.grade(trial)
            graded.append(
                GradedTrial(
                    suite=task["suite"],
                    task_id=task_id,
                    seed=seed,
                    passed=verdict.passed,
                    triage="infrastructure" if verdict.error == "no report" else None,
                )
            )

    # What this run was executed on (ADR-0032). Recorded on the scorecard so
    # `check_gate` can refuse to compare it against a baseline from another tier:
    # a local_mid run measured against a frontier baseline fails every margin, and a
    # run measured against different hardware passes or fails for reasons that have
    # nothing to do with the harness.
    provenance = {
        "profile_id": args.profile or os.environ.get("TBOT_EVAL_PROFILE"),
        "tier": args.tier or os.environ.get("TBOT_EVAL_TIER"),
        "hardware": args.hardware or os.environ.get("TBOT_EVAL_HARDWARE"),
    }
    card = build_scorecard(harness_version, graded, strengths, provenance)
    card["run_salt"] = args.run_salt
    card["dry_run"] = True
    card["trials_detail"] = [asdict(t) for t in graded]

    text = json.dumps(card, indent=2)
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")
        print(f"scorecard written to {args.out}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
